use serde::{Serialize, Deserialize};
use crate::{Element, DecimalElement};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountGuarantor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub party: Reference,
    #[serde(rename = "onHold")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_hold: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountCoverage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<PositiveInt>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(rename = "servicePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Vec<AccountCoverage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guarantor: Option<Vec<AccountGuarantor>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityDefinitionParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityDefinitionDynamicValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub expression: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionTiming {
    Timing(Timing),
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Range(Range),
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionProduct {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "doNotPerform")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "timingTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_timing: Option<Timing>,
    #[serde(rename = "timingDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_date_time: Option<DateTime>,
    #[serde(rename = "timingAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_age: Option<Age>,
    #[serde(rename = "timingPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_period: Option<Period>,
    #[serde(rename = "timingRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_range: Option<Range>,
    #[serde(rename = "timingDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<ActivityDefinitionParticipant>>,
    #[serde(rename = "productReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_reference: Option<Reference>,
    #[serde(rename = "productCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_codeable_concept: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<Dosage>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specimenRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen_requirement: Option<Vec<Reference>>,
    #[serde(rename = "observationRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_requirement: Option<Vec<Reference>>,
    #[serde(rename = "observationResultRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_result_requirement: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Canonical>,
    #[serde(rename = "dynamicValue")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_value: Option<Vec<ActivityDefinitionDynamicValue>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdverseEventSuspectEntity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub instance: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causality: Option<Vec<AdverseEventSuspectEntityCausality>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdverseEventSuspectEntityCausality {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment: Option<CodeableConcept>,
    #[serde(rename = "productRelatedness")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_relatedness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdverseEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "recordedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_date: Option<DateTime>,
    #[serde(rename = "resultingCondition")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "suspectEntity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspect_entity: Option<Vec<AdverseEventSuspectEntity>>,
    #[serde(rename = "subjectMedicalHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_medical_history: Option<Vec<Reference>>,
    #[serde(rename = "referenceDocument")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_document: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AllergyIntoleranceOnset {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllergyIntolerance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "clinicalStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clinical_status: Option<CodeableConcept>,
    #[serde(rename = "verificationStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<CodeableConcept>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "onsetDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_date_time: Option<DateTime>,
    #[serde(rename = "onsetAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_age: Option<Age>,
    #[serde(rename = "onsetPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_period: Option<Period>,
    #[serde(rename = "onsetRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_range: Option<Range>,
    #[serde(rename = "onsetString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_string: Option<String>,
    #[serde(rename = "recordedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asserter: Option<Reference>,
    #[serde(rename = "lastOccurrence")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "exposureRoute")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "cancelationReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancelation_reason: Option<CodeableConcept>,
    #[serde(rename = "serviceCategory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appointment_type: Option<CodeableConcept>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "supportingInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Instant>,
    #[serde(rename = "minutesDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes_duration: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "patientInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient_instruction: Option<String>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<AppointmentParticipant>>,
    #[serde(rename = "requestedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_period: Option<Vec<Period>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppointmentResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub appointment: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Instant>,
    #[serde(rename = "participantType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Reference>,
    #[serde(rename = "participantStatus")]
    pub participant_status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditEventEntityDetailValue {
    String(String),
    Base64Binary(Base64Binary),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventEntityDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Base64Binary,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub observer: Reference,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<Coding>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventAgentNetwork {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventAgent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who: Option<Reference>,
    #[serde(rename = "altId")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "purposeOfUse")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose_of_use: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEventEntity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub what: Option<Reference>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<Coding>,
    #[serde(rename = "securityLabel")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct AuditEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "outcomeDesc")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_desc: Option<String>,
    #[serde(rename = "purposeOfEvent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose_of_event: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Vec<AuditEventAgent>>,
    pub source: AuditEventSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<Vec<AuditEventEntity>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Basic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(rename = "contentType")]
    pub content_type: Code,
    #[serde(rename = "securityContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_context: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Base64Binary>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductCollectionCollected {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProductCollection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    #[serde(rename = "collectedDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected_date_time: Option<DateTime>,
    #[serde(rename = "collectedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected_period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductProcessingTime {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProductProcessing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Reference>,
    #[serde(rename = "timeDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_date_time: Option<DateTime>,
    #[serde(rename = "timePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProductStorage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductManipulationTime {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProductManipulation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "timeDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_date_time: Option<DateTime>,
    #[serde(rename = "timePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BiologicallyDerivedProduct {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "productCategory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_category: Option<Code>,
    #[serde(rename = "productCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodyStructure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub morphology: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<CodeableConcept>,
    #[serde(rename = "locationQualifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_qualifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<Attachment>>,
    pub patient: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleEntryResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(rename = "lastModified")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleEntryRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub method: Code,
    pub url: Uri,
    #[serde(rename = "ifNoneMatch")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_none_match: Option<String>,
    #[serde(rename = "ifModifiedSince")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_modified_since: Option<Instant>,
    #[serde(rename = "ifMatch")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_match: Option<String>,
    #[serde(rename = "ifNoneExist")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_none_exist: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: String,
    pub url: Uri,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<BundleLink>>,
    #[serde(rename = "fullUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct CapabilityStatementImplementation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custodian: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestResourceSearchParam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct CapabilityStatementMessagingEndpoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub protocol: Coding,
    pub address: Url,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementMessaging {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<CapabilityStatementMessagingEndpoint>>,
    #[serde(rename = "reliableCache")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reliable_cache: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
    #[serde(rename = "supportedMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_message: Option<Vec<CapabilityStatementMessagingSupportedMessage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementMessagingSupportedMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub definition: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
    pub profile: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestResourceOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub definition: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestInteraction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestSecurity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct CapabilityStatementSoftware {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "releaseDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "patchFormat")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_format: Option<Vec<Code>>,
    #[serde(rename = "implementationGuide")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct CapabilityStatementRestResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    #[serde(rename = "supportedProfile")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_profile: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<Vec<CapabilityStatementRestResourceInteraction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versioning: Option<Code>,
    #[serde(rename = "readHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_history: Option<Boolean>,
    #[serde(rename = "updateCreate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_create: Option<Boolean>,
    #[serde(rename = "conditionalCreate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_create: Option<Boolean>,
    #[serde(rename = "conditionalRead")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_read: Option<Code>,
    #[serde(rename = "conditionalUpdate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_update: Option<Boolean>,
    #[serde(rename = "conditionalDelete")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_delete: Option<Code>,
    #[serde(rename = "referencePolicy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_policy: Option<Vec<Code>>,
    #[serde(rename = "searchInclude")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_include: Option<Vec<String>>,
    #[serde(rename = "searchRevInclude")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_rev_include: Option<Vec<String>>,
    #[serde(rename = "searchParam")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRestResourceInteraction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityStatementRest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "searchParam")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment: Option<Vec<Canonical>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarePlanActivity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "outcomeCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_codeable_concept: Option<Vec<CodeableConcept>>,
    #[serde(rename = "outcomeReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct CarePlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "careTeam")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub care_team: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<Vec<CarePlanActivity>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanActivityDetailScheduled {
    Timing(Timing),
    Period(Period),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanActivityDetailProduct {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarePlanActivityDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Code>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "doNotPerform")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "scheduledTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_timing: Option<Timing>,
    #[serde(rename = "scheduledPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_period: Option<Period>,
    #[serde(rename = "scheduledString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "productCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "productReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_reference: Option<Reference>,
    #[serde(rename = "dailyAmount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_amount: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CareTeamParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<Reference>,
    #[serde(rename = "onBehalfOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CareTeam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "managingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntryRelatedEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationtype: Code,
    pub item: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    pub orderable: Boolean,
    #[serde(rename = "referencedItem")]
    pub referenced_item: Reference,
    #[serde(rename = "additionalIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "validityPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
    #[serde(rename = "validTo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<DateTime>,
    #[serde(rename = "lastUpdated")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<DateTime>,
    #[serde(rename = "additionalCharacteristic")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_characteristic: Option<Vec<CodeableConcept>>,
    #[serde(rename = "additionalClassification")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_classification: Option<Vec<CodeableConcept>>,
    #[serde(rename = "relatedEntry")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_entry: Option<Vec<CatalogEntryRelatedEntry>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeItemPerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemProduct {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "definitionUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_uri: Option<Vec<Uri>>,
    #[serde(rename = "definitionCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_canonical: Option<Vec<Canonical>>,
    pub status: Code,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_timing: Option<Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ChargeItemPerformer>>,
    #[serde(rename = "performingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performing_organization: Option<Reference>,
    #[serde(rename = "requestingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requesting_organization: Option<Reference>,
    #[serde(rename = "costCenter")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_center: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bodysite: Option<Vec<CodeableConcept>>,
    #[serde(rename = "factorOverride")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor_override: Option<Decimal>,
    #[serde(rename = "priceOverride")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_override: Option<Money>,
    #[serde(rename = "overrideReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    #[serde(rename = "enteredDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entered_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<Vec<Reference>>,
    #[serde(rename = "productReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_reference: Option<Reference>,
    #[serde(rename = "productCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_codeable_concept: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "supportingInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeItemDefinitionPropertyGroupPriceComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ChargeItemDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(rename = "derivedFromUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from_uri: Option<Vec<Uri>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[serde(rename = "propertyGroup")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub property_group: Option<Vec<ChargeItemDefinitionPropertyGroup>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChargeItemDefinitionApplicability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[serde(rename = "priceComponent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_component: Option<Vec<ChargeItemDefinitionPropertyGroupPriceComponent>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
    #[serde(rename = "preAuthRef")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[serde(rename = "claimResponse")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_response: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimSupportingInfoTiming {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimSupportingInfoValue {
    Boolean(Boolean),
    String(String),
    Quantity(Quantity),
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimSupportingInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "timingDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_date: Option<Date>,
    #[serde(rename = "timingPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_period: Option<Period>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimProcedureProcedure {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimProcedure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(rename = "procedureCodeableConcept")]
    pub procedure_codeable_concept: CodeableConcept,
    #[serde(rename = "procedureReference")]
    pub procedure_reference: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "programCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimDiagnosisDiagnosis {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "diagnosisCodeableConcept")]
    pub diagnosis_codeable_concept: CodeableConcept,
    #[serde(rename = "diagnosisReference")]
    pub diagnosis_reference: Reference,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "onAdmission")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_admission: Option<CodeableConcept>,
    #[serde(rename = "packageCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    #[serde(rename = "billablePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billable_period: Option<Period>,
    pub created: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurer: Option<Reference>,
    pub provider: Reference,
    pub priority: CodeableConcept,
    #[serde(rename = "fundsReserve")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funds_reserve: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<ClaimRelated>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prescription: Option<Reference>,
    #[serde(rename = "originalPrescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_prescription: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<ClaimPayee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(rename = "careTeam")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub care_team: Option<Vec<ClaimCareTeam>>,
    #[serde(rename = "supportingInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ClaimRelated {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimAccidentLocation {
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimAccident {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Date,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimPayee {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimItemServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimItemLocation {
    CodeableConcept(CodeableConcept),
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "careTeamSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "diagnosisSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "procedureSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "informationSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "servicedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_period: Option<Period>,
    #[serde(rename = "locationCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ClaimItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimCareTeam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ClaimItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "programCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "subDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ClaimItemDetailSubDetail>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseAddItemServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseAddItemLocation {
    CodeableConcept(CodeableConcept),
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseAddItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "detailSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "subdetailSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subdetail_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Vec<Reference>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "servicedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_period: Option<Period>,
    #[serde(rename = "locationCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ClaimResponseAddItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseAddItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(rename = "subDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ClaimResponseAddItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    pub item_sequence: PositiveInt,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ClaimResponseItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "preAuthRef")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<String>,
    #[serde(rename = "preAuthPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_auth_period: Option<Period>,
    #[serde(rename = "payeeType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<ClaimResponseItem>>,
    #[serde(rename = "addItem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_item: Option<Vec<ClaimResponseAddItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Vec<ClaimResponseTotal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<ClaimResponsePayment>,
    #[serde(rename = "fundsReserve")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funds_reserve: Option<CodeableConcept>,
    #[serde(rename = "formCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Attachment>,
    #[serde(rename = "processNote")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<ClaimResponseProcessNote>>,
    #[serde(rename = "communicationRequest")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication_request: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<ClaimResponseInsurance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Vec<ClaimResponseError>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseProcessNote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<PositiveInt>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseAddItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: PositiveInt,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseItemAdjudication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ClaimResponsePayment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Money>,
    #[serde(rename = "adjustmentReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "detailSequence")]
    pub detail_sequence: PositiveInt,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(rename = "subDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ClaimResponseItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseTotal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_sequence: Option<PositiveInt>,
    #[serde(rename = "detailSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_sequence: Option<PositiveInt>,
    #[serde(rename = "subDetailSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_detail_sequence: Option<PositiveInt>,
    pub code: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimResponseInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
    #[serde(rename = "claimResponse")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_response: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClinicalImpressionFinding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "itemReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClinicalImpressionEffective {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClinicalImpression {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "effectiveDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_date_time: Option<DateTime>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
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
    #[serde(rename = "prognosisCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prognosis_codeable_concept: Option<Vec<CodeableConcept>>,
    #[serde(rename = "prognosisReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prognosis_reference: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystemConceptDesignation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(rename = "use")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Coding>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystemProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum CodeSystemConceptPropertyValue {
    Code(Code),
    Coding(Coding),
    String(String),
    Integer(Integer),
    Boolean(Boolean),
    DateTime(DateTime),
    Decimal(Decimal),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystemConceptProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(rename = "valueCode")]
    pub value_code: Code,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSystem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "caseSensitive")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_sensitive: Option<Boolean>,
    #[serde(rename = "valueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Canonical>,
    #[serde(rename = "hierarchyMeaning")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchy_meaning: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compositional: Option<Boolean>,
    #[serde(rename = "versionNeeded")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct CodeSystemFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct CodeSystemConcept {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Communication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    #[serde(rename = "inResponseTo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_response_to: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Vec<CommunicationPayload>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationPayloadContent {
    String(String),
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentString")]
    pub content_string: String,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestPayloadContent {
    String(String),
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationRequestPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentString")]
    pub content_string: String,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestOccurrence {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "authoredOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompartmentDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct CompartmentDefinitionResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatesTo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relates_to: Option<Vec<CompositionRelatesTo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Vec<CompositionEvent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<Vec<CompositionSection>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompositionRelatesToTarget {
    Identifier(Identifier),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionRelatesTo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(rename = "targetIdentifier")]
    pub target_identifier: Identifier,
    #[serde(rename = "targetReference")]
    pub target_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionAttester {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionSection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "orderedBy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered_by: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<Vec<Reference>>,
    #[serde(rename = "emptyReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<Vec<CompositionSection>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMapGroupElementTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    pub equivalence: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "dependsOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapSource {
    Uri(Uri),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapTarget {
    Uri(Uri),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "sourceUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<Uri>,
    #[serde(rename = "sourceCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_canonical: Option<Canonical>,
    #[serde(rename = "targetUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_uri: Option<Uri>,
    #[serde(rename = "targetCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_canonical: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<ConceptMapGroup>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMapGroupElement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ConceptMapGroupElementTargetDependsOn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ConceptMapGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Uri>,
    #[serde(rename = "sourceVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Uri>,
    #[serde(rename = "targetVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Vec<ConceptMapGroupElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unmapped: Option<ConceptMapGroupUnmapped>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConceptMapGroupUnmapped {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Canonical>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionStage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment: Option<Vec<Reference>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConditionEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConditionOnset {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConditionAbatement {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "clinicalStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clinical_status: Option<CodeableConcept>,
    #[serde(rename = "verificationStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Vec<CodeableConcept>>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "onsetDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_date_time: Option<DateTime>,
    #[serde(rename = "onsetAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_age: Option<Age>,
    #[serde(rename = "onsetPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_period: Option<Period>,
    #[serde(rename = "onsetRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_range: Option<Range>,
    #[serde(rename = "onsetString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_string: Option<String>,
    #[serde(rename = "abatementDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abatement_date_time: Option<DateTime>,
    #[serde(rename = "abatementAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abatement_age: Option<Age>,
    #[serde(rename = "abatementPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abatement_period: Option<Period>,
    #[serde(rename = "abatementRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abatement_range: Option<Range>,
    #[serde(rename = "abatementString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abatement_string: Option<String>,
    #[serde(rename = "recordedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ConsentPolicy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentProvisionData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub meaning: Code,
    pub reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentVerification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub verified: Boolean,
    #[serde(rename = "verifiedWith")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_with: Option<Reference>,
    #[serde(rename = "verificationDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentProvision {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Vec<ConsentProvisionActor>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<CodeableConcept>>,
    #[serde(rename = "securityLabel")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "dataPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<ConsentProvisionData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provision: Option<Vec<ConsentProvision>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentProvisionActor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: CodeableConcept,
    pub reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConsentSource {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Consent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub scope: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient: Option<Reference>,
    #[serde(rename = "dateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Vec<Reference>>,
    #[serde(rename = "sourceAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_attachment: Option<Attachment>,
    #[serde(rename = "sourceReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<Vec<ConsentPolicy>>,
    #[serde(rename = "policyRule")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_rule: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification: Option<Vec<ConsentVerification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provision: Option<ConsentProvision>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTopic {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegallyBinding {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contract {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "legalState")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_state: Option<CodeableConcept>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Reference>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Uri>,
    #[serde(rename = "contentDerivative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_derivative: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies: Option<Period>,
    #[serde(rename = "expirationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "topicCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "topicReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_reference: Option<Reference>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "subType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "contentDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_definition: Option<ContractContentDefinition>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term: Option<Vec<ContractTerm>>,
    #[serde(rename = "supportingInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(rename = "relevantHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer: Option<Vec<ContractSigner>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendly: Option<Vec<ContractFriendly>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal: Option<Vec<ContractLegal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<Vec<ContractRule>>,
    #[serde(rename = "legallyBindingAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legally_binding_attachment: Option<Attachment>,
    #[serde(rename = "legallyBindingReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legally_binding_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermAssetContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegalContent {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractLegal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractRuleContent {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractSigner {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Coding,
    pub party: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<Signature>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermTopic {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTerm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies: Option<Period>,
    #[serde(rename = "topicCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "topicReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_reference: Option<Reference>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "subType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "securityLabel")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<ContractTermSecurityLabel>>,
    pub offer: ContractTermOffer,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asset: Option<Vec<ContractTermAsset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<ContractTermAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<ContractTerm>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractFriendlyContent {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractFriendly {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermOfferParty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Vec<Reference>>,
    pub role: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermActionOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "doNotPerform")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<ContractTermActionSubject>>,
    pub intent: CodeableConcept,
    #[serde(rename = "linkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    pub status: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "contextLinkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_link_id: Option<Vec<String>>,
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_timing: Option<Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Vec<Reference>>,
    #[serde(rename = "requesterLinkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester_link_id: Option<Vec<String>>,
    #[serde(rename = "performerType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "performerRole")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer_role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "performerLinkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer_link_id: Option<Vec<String>>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<String>>,
    #[serde(rename = "reasonLinkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_link_id: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "securityLabelNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermOfferAnswerValue {
    Boolean(Boolean),
    Decimal(Decimal),
    Integer(Integer),
    Date(Date),
    DateTime(DateTime),
    Time(Time),
    String(String),
    Uri(Uri),
    Attachment(Attachment),
    Coding(Coding),
    Quantity(Quantity),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermOfferAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueUri")]
    pub value_uri: Uri,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermOffer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Vec<ContractTermOfferParty>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Reference>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<CodeableConcept>,
    #[serde(rename = "decisionMode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_mode: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "linkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    #[serde(rename = "securityLabelNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermAsset {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<CodeableConcept>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "typeReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<ContractTermAssetContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(rename = "periodType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Vec<Period>>,
    #[serde(rename = "usePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_period: Option<Vec<Period>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "linkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    #[serde(rename = "securityLabelNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
    #[serde(rename = "valuedItem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valued_item: Option<Vec<ContractTermAssetValuedItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermActionSubject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractContentDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Reference>,
    #[serde(rename = "publicationDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<DateTime>,
    #[serde(rename = "publicationStatus")]
    pub publication_status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermAssetValuedItemEntity {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermAssetValuedItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "entityCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "entityReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "effectiveTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<String>,
    #[serde(rename = "paymentDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<Reference>,
    #[serde(rename = "linkId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_id: Option<Vec<String>>,
    #[serde(rename = "securityLabelNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractTermSecurityLabel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum CoverageCostToBeneficiaryValue {
    Quantity(Quantity),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageCostToBeneficiary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueMoney")]
    pub value_money: Money,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exception: Option<Vec<CoverageCostToBeneficiaryException>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageClass {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "policyHolder")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_holder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscriber: Option<Reference>,
    #[serde(rename = "subscriberId")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "costToBeneficiary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_to_beneficiary: Option<Vec<CoverageCostToBeneficiary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subrogation: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityRequestSupportingInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub information: Reference,
    #[serde(rename = "appliesToAll")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies_to_all: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityRequestInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal: Option<Boolean>,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_arrangement: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestItemDiagnosisDiagnosis {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityRequestItemDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "diagnosisCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "diagnosisReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    #[serde(rename = "servicedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_period: Option<Period>,
    pub created: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Reference>,
    pub insurer: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(rename = "supportingInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "supportingInfoSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_info_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_or_service: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inforce: Option<Boolean>,
    #[serde(rename = "benefitPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "authorizationRequired")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_required: Option<Boolean>,
    #[serde(rename = "authorizationSupporting")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_supporting: Option<Vec<CodeableConcept>>,
    #[serde(rename = "authorizationUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseInsuranceItemBenefitAllowed {
    UnsignedInt(UnsignedInt),
    String(String),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseInsuranceItemBenefitUsed {
    UnsignedInt(UnsignedInt),
    String(String),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityResponseInsuranceItemBenefit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "allowedUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "allowedString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_string: Option<String>,
    #[serde(rename = "allowedMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_money: Option<Money>,
    #[serde(rename = "usedUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "usedString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_string: Option<String>,
    #[serde(rename = "usedMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_money: Option<Money>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityResponseError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageEligibilityResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    #[serde(rename = "servicedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_period: Option<Period>,
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
    #[serde(rename = "preAuthRef")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Vec<CoverageEligibilityResponseError>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectedIssueEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectedIssueIdentified {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectedIssue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "identifiedDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identified_date_time: Option<DateTime>,
    #[serde(rename = "identifiedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identified_period: Option<Period>,
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
pub struct DeviceUdiCarrier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "deviceIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Uri>,
    #[serde(rename = "carrierAIDC")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carrier_a_i_d_c: Option<Base64Binary>,
    #[serde(rename = "carrierHRF")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carrier_h_r_f: Option<String>,
    #[serde(rename = "entryType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_type: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDeviceName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSpecialization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "systemType")]
    pub system_type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Identifier>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Reference>,
    #[serde(rename = "udiCarrier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi_carrier: Option<Vec<DeviceUdiCarrier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "distinctIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distinct_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(rename = "manufactureDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacture_date: Option<DateTime>,
    #[serde(rename = "expirationDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<DateTime>,
    #[serde(rename = "lotNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lot_number: Option<String>,
    #[serde(rename = "serialNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(rename = "deviceName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<Vec<DeviceDeviceName>>,
    #[serde(rename = "modelNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_number: Option<String>,
    #[serde(rename = "partNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_number: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct DeviceDefinitionMaterial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate: Option<Boolean>,
    #[serde(rename = "allergenicIndicator")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allergenic_indicator: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionSpecialization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "systemType")]
    pub system_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceDefinitionManufacturer {
    String(String),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "udiDeviceIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi_device_identifier: Option<Vec<DeviceDefinitionUdiDeviceIdentifier>>,
    #[serde(rename = "manufacturerString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer_string: Option<String>,
    #[serde(rename = "manufacturerReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer_reference: Option<Reference>,
    #[serde(rename = "deviceName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<Vec<DeviceDefinitionDeviceName>>,
    #[serde(rename = "modelNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_number: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialization: Option<Vec<DeviceDefinitionSpecialization>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety: Option<Vec<CodeableConcept>>,
    #[serde(rename = "shelfLifeStorage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    #[serde(rename = "physicalCharacteristics")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[serde(rename = "languageCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "onlineInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online_information: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "parentDevice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_device: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<Vec<DeviceDefinitionMaterial>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionUdiDeviceIdentifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "deviceIdentifier")]
    pub device_identifier: String,
    pub issuer: Uri,
    pub jurisdiction: Uri,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceDefinitionDeviceName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Code,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceMetric {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "operationalStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operational_status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Code>,
    pub category: Code,
    #[serde(rename = "measurementPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<Instant>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestCode {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "priorRequest")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_request: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    pub intent: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(rename = "codeReference")]
    pub code_reference: Reference,
    #[serde(rename = "codeCodeableConcept")]
    pub code_codeable_concept: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<DeviceRequestParameter>>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_timing: Option<Timing>,
    #[serde(rename = "authoredOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(rename = "performerType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "relevantHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestParameterValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Range(Range),
    Boolean(Boolean),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRequestParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "valueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceUseStatementTiming {
    Timing(Timing),
    Period(Period),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceUseStatement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    pub subject: Reference,
    #[serde(rename = "derivedFrom")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(rename = "timingTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_timing: Option<Timing>,
    #[serde(rename = "timingPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_period: Option<Period>,
    #[serde(rename = "timingDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_date_time: Option<DateTime>,
    #[serde(rename = "recordedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
    pub device: Reference,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub link: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticReportEffective {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "effectiveDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_date_time: Option<DateTime>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "resultsInterpreter")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results_interpreter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Vec<Reference>>,
    #[serde(rename = "imagingStudy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imaging_study: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<Vec<DiagnosticReportMedia>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    #[serde(rename = "conclusionCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "presentedForm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presented_form: Option<Vec<Attachment>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentManifestRelated {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "ref")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "masterIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct DocumentReferenceRelatesTo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "masterIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "docStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_status: Option<Code>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatesTo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relates_to: Option<Vec<DocumentReferenceRelatesTo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "securityLabel")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_label: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<DocumentReferenceContent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<DocumentReferenceContext>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentReferenceContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub attachment: Attachment,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<Coding>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentReferenceContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "facilityType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility_type: Option<CodeableConcept>,
    #[serde(rename = "practiceSetting")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub practice_setting: Option<CodeableConcept>,
    #[serde(rename = "sourcePatientInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_patient_info: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "synthesisType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesis_type: Option<CodeableConcept>,
    #[serde(rename = "studyType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_type: Option<CodeableConcept>,
    pub population: Reference,
    pub exposure: Reference,
    #[serde(rename = "exposureAlternative")]
    pub exposure_alternative: Reference,
    pub outcome: Reference,
    #[serde(rename = "sampleSize")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<EffectEvidenceSynthesisSampleSize>,
    #[serde(rename = "resultsByExposure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results_by_exposure: Option<Vec<EffectEvidenceSynthesisResultsByExposure>>,
    #[serde(rename = "effectEstimate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_estimate: Option<Vec<EffectEvidenceSynthesisEffectEstimate>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certainty: Option<Vec<EffectEvidenceSynthesisCertainty>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisEffectEstimate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "variantState")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_state: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(rename = "unitOfMeasure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "precisionEstimate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision_estimate: Option<Vec<EffectEvidenceSynthesisEffectEstimatePrecisionEstimate>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisEffectEstimatePrecisionEstimate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct EffectEvidenceSynthesisCertaintyCertaintySubcomponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisResultsByExposure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "exposureState")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure_state: Option<Code>,
    #[serde(rename = "variantState")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "certaintySubcomponent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certainty_subcomponent: Option<Vec<EffectEvidenceSynthesisCertaintyCertaintySubcomponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectEvidenceSynthesisSampleSize {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "numberOfStudies")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_studies: Option<Integer>,
    #[serde(rename = "numberOfParticipants")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_participants: Option<Integer>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterHospitalization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "preAdmissionIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_admission_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Reference>,
    #[serde(rename = "admitSource")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admit_source: Option<CodeableConcept>,
    #[serde(rename = "reAdmission")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub re_admission: Option<CodeableConcept>,
    #[serde(rename = "dietPreference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diet_preference: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specialCourtesy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_courtesy: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specialArrangement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_arrangement: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Reference>,
    #[serde(rename = "dischargeDisposition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discharge_disposition: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterStatusHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Encounter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_history: Option<Vec<EncounterStatusHistory>>,
    pub class: Coding,
    #[serde(rename = "classHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_history: Option<Vec<EncounterClassHistory>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(rename = "episodeOfCare")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode_of_care: Option<Vec<Reference>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<EncounterParticipant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appointment: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Duration>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<EncounterDiagnosis>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hospitalization: Option<EncounterHospitalization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<EncounterLocation>>,
    #[serde(rename = "serviceProvider")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_provider: Option<Reference>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub individual: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub location: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "physicalType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterClassHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub class: Coding,
    pub period: Period,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncounterDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    #[serde(rename = "use")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#use: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<PositiveInt>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "connectionType")]
    pub connection_type: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "managingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "payloadType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "payloadMimeType")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "requestProvider")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_provider: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeOfCareDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<PositiveInt>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeOfCareStatusHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_history: Option<Vec<EpisodeOfCareStatusHistory>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<Vec<EpisodeOfCareDiagnosis>>,
    pub patient: Reference,
    #[serde(rename = "managingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "referralRequest")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral_request: Option<Vec<Reference>>,
    #[serde(rename = "careManager")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub care_manager: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "shortTitle")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "exposureBackground")]
    pub exposure_background: Reference,
    #[serde(rename = "exposureVariant")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "shortTitle")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristic: Option<Vec<EvidenceVariableCharacteristic>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableCharacteristicDefinition {
    Reference(Reference),
    Canonical(Canonical),
    CodeableConcept(CodeableConcept),
    Expression(Expression),
    DataRequirement(DataRequirement),
    TriggerDefinition(TriggerDefinition),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableCharacteristicParticipantEffective {
    DateTime(DateTime),
    Period(Period),
    Duration(Duration),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceVariableCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "definitionReference")]
    pub definition_reference: Reference,
    #[serde(rename = "definitionCanonical")]
    pub definition_canonical: Canonical,
    #[serde(rename = "definitionCodeableConcept")]
    pub definition_codeable_concept: CodeableConcept,
    #[serde(rename = "definitionExpression")]
    pub definition_expression: Expression,
    #[serde(rename = "definitionDataRequirement")]
    pub definition_data_requirement: DataRequirement,
    #[serde(rename = "definitionTriggerDefinition")]
    pub definition_trigger_definition: TriggerDefinition,
    #[serde(rename = "usageContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Boolean>,
    #[serde(rename = "participantEffectiveDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_date_time: Option<DateTime>,
    #[serde(rename = "participantEffectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_period: Option<Period>,
    #[serde(rename = "participantEffectiveDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_duration: Option<Duration>,
    #[serde(rename = "participantEffectiveTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_timing: Option<Timing>,
    #[serde(rename = "timeFromStart")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_from_start: Option<Duration>,
    #[serde(rename = "groupMeasure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_measure: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioProcessStepAlternative {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioActor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ExampleScenarioInstanceVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "versionId")]
    pub version_id: String,
    pub description: Markdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioInstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "containedInstance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained_instance: Option<Vec<ExampleScenarioInstanceContainedInstance>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioInstanceContainedInstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "resourceId")]
    pub resource_id: String,
    #[serde(rename = "versionId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioProcess {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "preConditions")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_conditions: Option<Markdown>,
    #[serde(rename = "postConditions")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_conditions: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExampleScenarioProcessStep {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ExampleScenario {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ExampleScenarioProcessStepOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: String,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "initiatorActive")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator_active: Option<Boolean>,
    #[serde(rename = "receiverActive")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<ExampleScenarioInstanceContainedInstance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<ExampleScenarioInstanceContainedInstance>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitTotal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitInsurance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(rename = "preAuthRef")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitCareTeam {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitSupportingInfoTiming {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitSupportingInfoValue {
    Boolean(Boolean),
    String(String),
    Quantity(Quantity),
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitSupportingInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "timingDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_date: Option<Date>,
    #[serde(rename = "timingPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_period: Option<Period>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Coding>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitPayment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Money>,
    #[serde(rename = "adjustmentReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjustment_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitDiagnosisDiagnosis {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitDiagnosis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "diagnosisCodeableConcept")]
    pub diagnosis_codeable_concept: CodeableConcept,
    #[serde(rename = "diagnosisReference")]
    pub diagnosis_reference: Reference,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "onAdmission")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_admission: Option<CodeableConcept>,
    #[serde(rename = "packageCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitItemAdjudication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ExplanationOfBenefitProcessNote {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<PositiveInt>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    #[serde(rename = "billablePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billable_period: Option<Period>,
    pub created: DateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterer: Option<Reference>,
    pub insurer: Reference,
    pub provider: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    #[serde(rename = "fundsReserveRequested")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funds_reserve_requested: Option<CodeableConcept>,
    #[serde(rename = "fundsReserve")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funds_reserve: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related: Option<Vec<ExplanationOfBenefitRelated>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prescription: Option<Reference>,
    #[serde(rename = "originalPrescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_prescription: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<ExplanationOfBenefitPayee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub facility: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<Reference>,
    #[serde(rename = "claimResponse")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_response: Option<Reference>,
    pub outcome: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(rename = "preAuthRef")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[serde(rename = "preAuthRefPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_auth_ref_period: Option<Vec<Period>>,
    #[serde(rename = "careTeam")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub care_team: Option<Vec<ExplanationOfBenefitCareTeam>>,
    #[serde(rename = "supportingInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "addItem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_item: Option<Vec<ExplanationOfBenefitAddItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<Vec<ExplanationOfBenefitTotal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<ExplanationOfBenefitPayment>,
    #[serde(rename = "formCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form: Option<Attachment>,
    #[serde(rename = "processNote")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<ExplanationOfBenefitProcessNote>>,
    #[serde(rename = "benefitPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit_period: Option<Period>,
    #[serde(rename = "benefitBalance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit_balance: Option<Vec<ExplanationOfBenefitBenefitBalance>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "programCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitItemServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitItemLocation {
    CodeableConcept(CodeableConcept),
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "careTeamSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "diagnosisSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "procedureSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "informationSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revenue: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "servicedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_period: Option<Period>,
    #[serde(rename = "locationCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ExplanationOfBenefitItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitRelated {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ExplanationOfBenefitAddItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(rename = "subDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitAddItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAddItemServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAddItemLocation {
    CodeableConcept(CodeableConcept),
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitAddItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "detailSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "subDetailSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<Vec<Reference>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "servicedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serviced_period: Option<Period>,
    #[serde(rename = "locationCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<ExplanationOfBenefitAddItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitPayee {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitProcedureProcedure {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitProcedure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(rename = "procedureCodeableConcept")]
    pub procedure_codeable_concept: CodeableConcept,
    #[serde(rename = "procedureReference")]
    pub procedure_reference: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitAddItemDetailSubDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitBenefitBalance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAccidentLocation {
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitAccident {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialAllowed {
    UnsignedInt(UnsignedInt),
    String(String),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialUsed {
    UnsignedInt(UnsignedInt),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitBenefitBalanceFinancial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "allowedUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "allowedString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_string: Option<String>,
    #[serde(rename = "allowedMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_money: Option<Money>,
    #[serde(rename = "usedUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "usedMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_money: Option<Money>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplanationOfBenefitItemDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "programCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(rename = "subDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitItemDetailSubDetail>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryConditionOnset {
    Age(Age),
    Range(Range),
    Period(Period),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyMemberHistoryCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(rename = "contributedToDeath")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributed_to_death: Option<Boolean>,
    #[serde(rename = "onsetAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_age: Option<Age>,
    #[serde(rename = "onsetRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_range: Option<Range>,
    #[serde(rename = "onsetPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_period: Option<Period>,
    #[serde(rename = "onsetString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onset_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryBorn {
    Period(Period),
    Date(Date),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryAge {
    Age(Age),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryDeceased {
    Boolean(Boolean),
    Age(Age),
    Range(Range),
    Date(Date),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyMemberHistory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    pub status: Code,
    #[serde(rename = "dataAbsentReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub relationship: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sex: Option<CodeableConcept>,
    #[serde(rename = "bornPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub born_period: Option<Period>,
    #[serde(rename = "bornDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub born_date: Option<Date>,
    #[serde(rename = "bornString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub born_string: Option<String>,
    #[serde(rename = "ageAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_age: Option<Age>,
    #[serde(rename = "ageRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_range: Option<Range>,
    #[serde(rename = "ageString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_string: Option<String>,
    #[serde(rename = "estimatedAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_age: Option<Boolean>,
    #[serde(rename = "deceasedBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deceased_boolean: Option<Boolean>,
    #[serde(rename = "deceasedAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deceased_age: Option<Age>,
    #[serde(rename = "deceasedRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deceased_range: Option<Range>,
    #[serde(rename = "deceasedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deceased_date: Option<Date>,
    #[serde(rename = "deceasedString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deceased_string: Option<String>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<FamilyMemberHistoryCondition>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flag {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalStart {
    Date(Date),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "lifecycleStatus")]
    pub lifecycle_status: Code,
    #[serde(rename = "achievementStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub achievement_status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub subject: Reference,
    #[serde(rename = "startDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<Date>,
    #[serde(rename = "startCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_codeable_concept: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<GoalTarget>>,
    #[serde(rename = "statusDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_date: Option<Date>,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(rename = "expressedBy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expressed_by: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "outcomeCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "outcomeReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_reference: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalTargetDetail {
    Quantity(Quantity),
    Range(Range),
    CodeableConcept(CodeableConcept),
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Ratio(Ratio),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalTargetDue {
    Date(Date),
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure: Option<CodeableConcept>,
    #[serde(rename = "detailQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_quantity: Option<Quantity>,
    #[serde(rename = "detailRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_range: Option<Range>,
    #[serde(rename = "detailCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "detailString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_string: Option<String>,
    #[serde(rename = "detailBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_boolean: Option<Boolean>,
    #[serde(rename = "detailInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_integer: Option<Integer>,
    #[serde(rename = "detailRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_ratio: Option<Ratio>,
    #[serde(rename = "dueDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<Date>,
    #[serde(rename = "dueDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_duration: Option<Duration>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDefinitionLinkTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphDefinitionLink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "sliceName")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct GraphDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct Group {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "managingEntity")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub entity: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inactive: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupCharacteristicValue {
    CodeableConcept(CodeableConcept),
    Boolean(Boolean),
    Quantity(Quantity),
    Range(Range),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    pub exclude: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuidanceResponseModule {
    Uri(Uri),
    Canonical(Canonical),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "requestIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "moduleUri")]
    pub module_uri: Uri,
    #[serde(rename = "moduleCanonical")]
    pub module_canonical: Canonical,
    #[serde(rename = "moduleCodeableConcept")]
    pub module_codeable_concept: CodeableConcept,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "evaluationMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation_message: Option<Vec<Reference>>,
    #[serde(rename = "outputParameters")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_parameters: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Reference>,
    #[serde(rename = "dataRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_requirement: Option<Vec<DataRequirement>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthcareServiceNotAvailable {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "availableStartTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_start_time: Option<Time>,
    #[serde(rename = "availableEndTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_end_time: Option<Time>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthcareServiceEligibility {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(rename = "providedBy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provided_by: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "extraDetails")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_details: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Attachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(rename = "coverageArea")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_area: Option<Vec<Reference>>,
    #[serde(rename = "serviceProvisionCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_provision_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligibility: Option<Vec<HealthcareServiceEligibility>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristic: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referralMethod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral_method: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentRequired")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appointment_required: Option<Boolean>,
    #[serde(rename = "availableTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_time: Option<Vec<HealthcareServiceAvailableTime>>,
    #[serde(rename = "notAvailable")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_available: Option<Vec<HealthcareServiceNotAvailable>>,
    #[serde(rename = "availabilityExceptions")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_exceptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagingStudySeries {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<UnsignedInt>,
    pub modality: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "numberOfInstances")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_instances: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referrer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpreter: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
    #[serde(rename = "numberOfSeries")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_series: Option<UnsignedInt>,
    #[serde(rename = "numberOfInstances")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_instances: Option<UnsignedInt>,
    #[serde(rename = "procedureReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure_reference: Option<Reference>,
    #[serde(rename = "procedureCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure_code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagingStudySeriesInstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ImmunizationReaction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationOccurrence {
    DateTime(DateTime),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Immunization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "vaccineCode")]
    pub vaccine_code: CodeableConcept,
    pub patient: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: DateTime,
    #[serde(rename = "occurrenceString")]
    pub occurrence_string: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded: Option<DateTime>,
    #[serde(rename = "primarySource")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_source: Option<Boolean>,
    #[serde(rename = "reportOrigin")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_origin: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Reference>,
    #[serde(rename = "lotNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lot_number: Option<String>,
    #[serde(rename = "expirationDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<CodeableConcept>,
    #[serde(rename = "doseQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ImmunizationPerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "isSubpotent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_subpotent: Option<Boolean>,
    #[serde(rename = "subpotentReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subpotent_reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub education: Option<Vec<ImmunizationEducation>>,
    #[serde(rename = "programEligibility")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub program_eligibility: Option<Vec<CodeableConcept>>,
    #[serde(rename = "fundingSource")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding_source: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction: Option<Vec<ImmunizationReaction>>,
    #[serde(rename = "protocolApplied")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_applied: Option<Vec<ImmunizationProtocolApplied>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationEducation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "documentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Uri>,
    #[serde(rename = "publicationDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<DateTime>,
    #[serde(rename = "presentationDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationProtocolAppliedDoseNumber {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationProtocolAppliedSeriesDoses {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationProtocolApplied {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<Reference>,
    #[serde(rename = "targetDisease")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_disease: Option<Vec<CodeableConcept>>,
    #[serde(rename = "doseNumberPositiveInt")]
    pub dose_number_positive_int: PositiveInt,
    #[serde(rename = "doseNumberString")]
    pub dose_number_string: String,
    #[serde(rename = "seriesDosesPositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_doses_positive_int: Option<PositiveInt>,
    #[serde(rename = "seriesDosesString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_doses_string: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationPerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationDoseNumber {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationSeriesDoses {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationEvaluation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "doseStatusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_status_reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(rename = "doseNumberPositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_number_positive_int: Option<PositiveInt>,
    #[serde(rename = "doseNumberString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_number_string: Option<String>,
    #[serde(rename = "seriesDosesPositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_doses_positive_int: Option<PositiveInt>,
    #[serde(rename = "seriesDosesString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_doses_string: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationRecommendationRecommendationDateCriterion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: DateTime,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationRecommendation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationRecommendationDoseNumber {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationRecommendationSeriesDoses {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmunizationRecommendationRecommendation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "vaccineCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vaccine_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "targetDisease")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_disease: Option<CodeableConcept>,
    #[serde(rename = "contraindicatedVaccineCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contraindicated_vaccine_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "forecastStatus")]
    pub forecast_status: CodeableConcept,
    #[serde(rename = "forecastReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forecast_reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "dateCriterion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_criterion: Option<Vec<ImmunizationRecommendationRecommendationDateCriterion>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    #[serde(rename = "doseNumberPositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_number_positive_int: Option<PositiveInt>,
    #[serde(rename = "doseNumberString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_number_string: Option<String>,
    #[serde(rename = "seriesDosesPositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_doses_positive_int: Option<PositiveInt>,
    #[serde(rename = "seriesDosesString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_doses_string: Option<String>,
    #[serde(rename = "supportingImmunization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_immunization: Option<Vec<Reference>>,
    #[serde(rename = "supportingPatientInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_patient_information: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideDefinitionPageName {
    Url(Url),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionPage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "nameUrl")]
    pub name_url: Url,
    #[serde(rename = "nameReference")]
    pub name_reference: Reference,
    pub title: String,
    pub generation: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Vec<ImplementationGuideDefinitionPage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideManifestResourceExample {
    Boolean(Boolean),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideManifestResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[serde(rename = "exampleBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_boolean: Option<Boolean>,
    #[serde(rename = "exampleCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_canonical: Option<Canonical>,
    #[serde(rename = "relativePath")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<Url>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ImplementationGuide {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "packageId")]
    pub package_id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<Code>,
    #[serde(rename = "fhirVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fhir_version: Option<Vec<Code>>,
    #[serde(rename = "dependsOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ImplementationGuideManifestPage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionTemplate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideGlobal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub profile: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideDefinitionResourceExample {
    Boolean(Boolean),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[serde(rename = "fhirVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fhir_version: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "exampleBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_boolean: Option<Boolean>,
    #[serde(rename = "exampleCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_canonical: Option<Canonical>,
    #[serde(rename = "groupingId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping_id: Option<Id>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDefinitionParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideManifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ImplementationGuideDefinitionGrouping {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationGuideDependsOn {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Canonical,
    #[serde(rename = "packageId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanCoverageBenefitLimit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct InsurancePlanPlanGeneralCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "groupSize")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_size: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanContact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct InsurancePlanPlanSpecificCostBenefit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Vec<InsurancePlanPlanSpecificCostBenefitCost>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanCoverageBenefit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct InsurancePlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(rename = "ownedBy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<Reference>,
    #[serde(rename = "administeredBy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub administered_by: Option<Reference>,
    #[serde(rename = "coverageArea")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct InsurancePlanPlanSpecificCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub benefit: Option<Vec<InsurancePlanPlanSpecificCostBenefit>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsurancePlanPlanSpecificCostBenefitCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct InsurancePlanPlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "coverageArea")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_area: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<Reference>>,
    #[serde(rename = "generalCost")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub general_cost: Option<Vec<InsurancePlanPlanGeneralCost>>,
    #[serde(rename = "specificCost")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specific_cost: Option<Vec<InsurancePlanPlanSpecificCost>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invoice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "cancelledReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancelled_reason: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "lineItem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_item: Option<Vec<InvoiceLineItem>>,
    #[serde(rename = "totalPriceComponent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
    #[serde(rename = "totalNet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_net: Option<Money>,
    #[serde(rename = "totalGross")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_gross: Option<Money>,
    #[serde(rename = "paymentTerms")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_terms: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvoiceLineItemChargeItem {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceLineItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<PositiveInt>,
    #[serde(rename = "chargeItemReference")]
    pub charge_item_reference: Reference,
    #[serde(rename = "chargeItemCodeableConcept")]
    pub charge_item_codeable_concept: CodeableConcept,
    #[serde(rename = "priceComponent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceLineItemPriceComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum LibrarySubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<ParameterDefinition>>,
    #[serde(rename = "dataRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "orderedBy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordered_by: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<Vec<ListEntry>>,
    #[serde(rename = "emptyReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_reason: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct LocationHoursOfOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "openingTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening_time: Option<Time>,
    #[serde(rename = "closingTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closing_time: Option<Time>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationPosition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub longitude: Decimal,
    pub latitude: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub altitude: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "operationalStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operational_status: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Code>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    #[serde(rename = "physicalType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<LocationPosition>,
    #[serde(rename = "managingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
    #[serde(rename = "hoursOfOperation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hours_of_operation: Option<Vec<LocationHoursOfOperation>>,
    #[serde(rename = "availabilityExceptions")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_exceptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Measure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disclaimer: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoring: Option<CodeableConcept>,
    #[serde(rename = "compositeScoring")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite_scoring: Option<CodeableConcept>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "riskAdjustment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_adjustment: Option<String>,
    #[serde(rename = "rateAggregation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_aggregation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<Markdown>,
    #[serde(rename = "clinicalRecommendationStatement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clinical_recommendation_statement: Option<Markdown>,
    #[serde(rename = "improvementNotation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub improvement_notation: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Vec<Markdown>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<MeasureGroup>>,
    #[serde(rename = "supplementalData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplemental_data: Option<Vec<MeasureSupplementalData>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureGroupStratifierComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct MeasureSupplementalData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct MeasureGroupPopulation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureGroupStratifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct MeasureReportGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<MeasureReportGroupPopulation>>,
    #[serde(rename = "measureScore")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure_score: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stratifier: Option<Vec<MeasureReportGroupStratifier>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupStratifierStratumComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupStratifierStratumPopulation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<Integer>,
    #[serde(rename = "subjectResults")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_results: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupStratifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stratum: Option<Vec<MeasureReportGroupStratifierStratum>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupPopulation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<Integer>,
    #[serde(rename = "subjectResults")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_results: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReportGroupStratifierStratum {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<MeasureReportGroupStratifierStratumComponent>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<MeasureReportGroupStratifierStratumPopulation>>,
    #[serde(rename = "measureScore")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure_score: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasureReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "improvementNotation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub improvement_notation: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Vec<MeasureReportGroup>>,
    #[serde(rename = "evaluatedResource")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluated_resource: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaCreated {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modality: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "createdDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_date_time: Option<DateTime>,
    #[serde(rename = "createdPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "deviceName")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct MedicationBatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "lotNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lot_number: Option<String>,
    #[serde(rename = "expirationDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration_date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Medication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationIngredientItem {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationIngredient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: CodeableConcept,
    #[serde(rename = "itemReference")]
    pub item_reference: Reference,
    #[serde(rename = "isActive")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Ratio>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationEffective {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationAdministration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates: Option<Vec<Uri>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "supportingInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(rename = "effectiveDateTime")]
    pub effective_date_time: DateTime,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Period,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<MedicationAdministrationPerformer>>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<MedicationAdministrationDosage>,
    #[serde(rename = "eventHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationAdministrationPerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationDosageRate {
    Ratio(Ratio),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationAdministrationDosage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "rateRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_ratio: Option<Ratio>,
    #[serde(rename = "rateQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_quantity: Option<Quantity>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationDispenseSubstitution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "wasSubstituted")]
    pub was_substituted: Boolean,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "responsibleParty")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsible_party: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationDispensePerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseStatusReason {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationDispense {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReasonCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "statusReasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "supportingInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<MedicationDispensePerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(rename = "authorizingPrescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorizing_prescription: Option<Vec<Reference>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "daysSupply")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_supply: Option<Quantity>,
    #[serde(rename = "whenPrepared")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_prepared: Option<DateTime>,
    #[serde(rename = "whenHandedOver")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_handed_over: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "dosageInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<MedicationDispenseSubstitution>,
    #[serde(rename = "detectedIssue")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_issue: Option<Vec<Reference>>,
    #[serde(rename = "eventHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeMonograph {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeMedicineClassification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgePackaging {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRegulatory {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "regulatoryAuthority")]
    pub regulatory_authority: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<Vec<MedicationKnowledgeRegulatorySubstitution>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<MedicationKnowledgeRegulatorySchedule>>,
    #[serde(rename = "maxDispense")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dispense: Option<MedicationKnowledgeRegulatoryMaxDispense>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Reference>,
    #[serde(rename = "doseForm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_form: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synonym: Option<Vec<String>>,
    #[serde(rename = "relatedMedicationKnowledge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_medication_knowledge: Option<Vec<MedicationKnowledgeRelatedMedicationKnowledge>>,
    #[serde(rename = "associatedMedication")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub associated_medication: Option<Vec<Reference>>,
    #[serde(rename = "productType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monograph: Option<Vec<MedicationKnowledgeMonograph>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<MedicationKnowledgeIngredient>>,
    #[serde(rename = "preparationInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparation_instruction: Option<Markdown>,
    #[serde(rename = "intendedRoute")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intended_route: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<Vec<MedicationKnowledgeCost>>,
    #[serde(rename = "monitoringProgram")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monitoring_program: Option<Vec<MedicationKnowledgeMonitoringProgram>>,
    #[serde(rename = "administrationGuidelines")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub administration_guidelines: Option<Vec<MedicationKnowledgeAdministrationGuidelines>>,
    #[serde(rename = "medicineClassification")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medicine_classification: Option<Vec<MedicationKnowledgeMedicineClassification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packaging: Option<MedicationKnowledgePackaging>,
    #[serde(rename = "drugCharacteristic")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drug_characteristic: Option<Vec<MedicationKnowledgeDrugCharacteristic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contraindication: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulatory: Option<Vec<MedicationKnowledgeRegulatory>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinetics: Option<Vec<MedicationKnowledgeKinetics>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRelatedMedicationKnowledge {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRegulatorySubstitution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub allowed: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeAdministrationGuidelinesPatientCharacteristicsCharacteristic {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "characteristicCodeableConcept")]
    pub characteristic_codeable_concept: CodeableConcept,
    #[serde(rename = "characteristicQuantity")]
    pub characteristic_quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRegulatorySchedule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeAdministrationGuidelinesDosage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<Dosage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeRegulatoryMaxDispense {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeAdministrationGuidelinesIndication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeAdministrationGuidelines {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<MedicationKnowledgeAdministrationGuidelinesDosage>>,
    #[serde(rename = "indicationCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indication_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "indicationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indication_reference: Option<Reference>,
    #[serde(rename = "patientCharacteristics")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient_characteristics: Option<Vec<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeMonitoringProgram {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeIngredientItem {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeIngredient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: CodeableConcept,
    #[serde(rename = "itemReference")]
    pub item_reference: Reference,
    #[serde(rename = "isActive")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength: Option<Ratio>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeDrugCharacteristicValue {
    CodeableConcept(CodeableConcept),
    String(String),
    Quantity(Quantity),
    Base64Binary(Base64Binary),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeDrugCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "valueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueBase64Binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_base64_binary: Option<Base64Binary>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeKinetics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "areaUnderCurve")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_under_curve: Option<Vec<Quantity>>,
    #[serde(rename = "lethalDose50")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lethal_dose50: Option<Vec<Quantity>>,
    #[serde(rename = "halfLifePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub half_life_period: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationKnowledgeCost {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub cost: Money,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestReported {
    Boolean(Boolean),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    pub intent: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "reportedBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_boolean: Option<Boolean>,
    #[serde(rename = "reportedReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_reference: Option<Reference>,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "supportingInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(rename = "authoredOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "performerType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    #[serde(rename = "courseOfTherapyType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_of_therapy_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "dosageInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    #[serde(rename = "dispenseRequest")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispense_request: Option<MedicationRequestDispenseRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<MedicationRequestSubstitution>,
    #[serde(rename = "priorPrescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prior_prescription: Option<Reference>,
    #[serde(rename = "detectedIssue")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detected_issue: Option<Vec<Reference>>,
    #[serde(rename = "eventHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationRequestDispenseRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "initialFill")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_fill: Option<MedicationRequestDispenseRequestInitialFill>,
    #[serde(rename = "dispenseInterval")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispense_interval: Option<Duration>,
    #[serde(rename = "validityPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
    #[serde(rename = "numberOfRepeatsAllowed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_repeats_allowed: Option<UnsignedInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "expectedSupplyDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_supply_duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestSubstitutionAllowed {
    Boolean(Boolean),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationRequestSubstitution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "allowedBoolean")]
    pub allowed_boolean: Boolean,
    #[serde(rename = "allowedCodeableConcept")]
    pub allowed_codeable_concept: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationRequestDispenseRequestInitialFill {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementEffective {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicationStatement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Reference>,
    #[serde(rename = "effectiveDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_date_time: Option<DateTime>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(rename = "dateAsserted")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_asserted: Option<DateTime>,
    #[serde(rename = "informationSource")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub information_source: Option<Reference>,
    #[serde(rename = "derivedFrom")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage: Option<Vec<Dosage>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductNameCountryLanguage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<CodeableConcept>,
    pub language: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProduct {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<Coding>,
    #[serde(rename = "combinedPharmaceuticalDoseForm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combined_pharmaceutical_dose_form: Option<CodeableConcept>,
    #[serde(rename = "legalStatusOfSupply")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[serde(rename = "additionalMonitoringIndicator")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_monitoring_indicator: Option<CodeableConcept>,
    #[serde(rename = "specialMeasures")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_measures: Option<Vec<String>>,
    #[serde(rename = "paediatricUseIndicator")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paediatric_use_indicator: Option<CodeableConcept>,
    #[serde(rename = "productClassification")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_classification: Option<Vec<CodeableConcept>>,
    #[serde(rename = "marketingStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    #[serde(rename = "pharmaceuticalProduct")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pharmaceutical_product: Option<Vec<Reference>>,
    #[serde(rename = "packagedMedicinalProduct")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packaged_medicinal_product: Option<Vec<Reference>>,
    #[serde(rename = "attachedDocument")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attached_document: Option<Vec<Reference>>,
    #[serde(rename = "masterFile")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_file: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<Reference>>,
    #[serde(rename = "clinicalTrial")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clinical_trial: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<MedicinalProductName>>,
    #[serde(rename = "crossReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_reference: Option<Vec<Identifier>>,
    #[serde(rename = "manufacturingBusinessOperation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturing_business_operation: Option<Vec<MedicinalProductManufacturingBusinessOperation>>,
    #[serde(rename = "specialDesignation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special_designation: Option<Vec<MedicinalProductSpecialDesignation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductNameNamePart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub part: String,
    #[serde(rename = "type")]
    pub r#type: Coding,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductManufacturingBusinessOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "operationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_type: Option<CodeableConcept>,
    #[serde(rename = "authorisationReferenceNumber")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorisation_reference_number: Option<Identifier>,
    #[serde(rename = "effectiveDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_date: Option<DateTime>,
    #[serde(rename = "confidentialityIndicator")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidentiality_indicator: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulator: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductSpecialDesignationIndication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductSpecialDesignation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "intendedUse")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intended_use: Option<CodeableConcept>,
    #[serde(rename = "indicationCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indication_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "indicationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indication_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub species: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productName")]
    pub product_name: String,
    #[serde(rename = "namePart")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_part: Option<Vec<MedicinalProductNameNamePart>>,
    #[serde(rename = "countryLanguage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_language: Option<Vec<MedicinalProductNameCountryLanguage>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductAuthorization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "statusDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_date: Option<DateTime>,
    #[serde(rename = "restoreDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_date: Option<DateTime>,
    #[serde(rename = "validityPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
    #[serde(rename = "dataExclusivityPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_exclusivity_period: Option<Period>,
    #[serde(rename = "dateOfFirstAuthorization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_of_first_authorization: Option<DateTime>,
    #[serde(rename = "internationalBirthDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub international_birth_date: Option<DateTime>,
    #[serde(rename = "legalBasis")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_basis: Option<CodeableConcept>,
    #[serde(rename = "jurisdictionalAuthorization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdictional_authorization: Option<Vec<MedicinalProductAuthorizationJurisdictionalAuthorization>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub holder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regulator: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<MedicinalProductAuthorizationProcedure>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductAuthorizationProcedureDate {
    Period(Period),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductAuthorizationProcedure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "datePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_period: Option<Period>,
    #[serde(rename = "dateDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_date_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application: Option<Vec<MedicinalProductAuthorizationProcedure>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductAuthorizationJurisdictionalAuthorization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(rename = "legalStatusOfSupply")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[serde(rename = "validityPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity_period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductContraindication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disease: Option<CodeableConcept>,
    #[serde(rename = "diseaseStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disease_status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comorbidity: Option<Vec<CodeableConcept>>,
    #[serde(rename = "therapeuticIndication")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub therapeutic_indication: Option<Vec<Reference>>,
    #[serde(rename = "otherTherapy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_therapy: Option<Vec<MedicinalProductContraindicationOtherTherapy>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<Population>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductContraindicationOtherTherapyMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductContraindicationOtherTherapy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "therapyRelationshipType")]
    pub therapy_relationship_type: CodeableConcept,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductIndicationOtherTherapyMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIndicationOtherTherapy {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "therapyRelationshipType")]
    pub therapy_relationship_type: CodeableConcept,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIndication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(rename = "diseaseSymptomProcedure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disease_symptom_procedure: Option<CodeableConcept>,
    #[serde(rename = "diseaseStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disease_status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comorbidity: Option<Vec<CodeableConcept>>,
    #[serde(rename = "intendedEffect")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intended_effect: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Quantity>,
    #[serde(rename = "otherTherapy")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_therapy: Option<Vec<MedicinalProductIndicationOtherTherapy>>,
    #[serde(rename = "undesirableEffect")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undesirable_effect: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<Population>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIngredientSubstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub presentation: Ratio,
    #[serde(rename = "presentationLowLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_low_limit: Option<Ratio>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration: Option<Ratio>,
    #[serde(rename = "concentrationLowLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concentration_low_limit: Option<Ratio>,
    #[serde(rename = "measurementPoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referenceStrength")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_strength: Option<Vec<MedicinalProductIngredientSpecifiedSubstanceStrengthReferenceStrength>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIngredientSpecifiedSubstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct MedicinalProductIngredient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    pub role: CodeableConcept,
    #[serde(rename = "allergenicIndicator")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allergenic_indicator: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
    #[serde(rename = "specifiedSubstance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specified_substance: Option<Vec<MedicinalProductIngredientSpecifiedSubstance>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance: Option<MedicinalProductIngredientSubstance>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductIngredientSpecifiedSubstanceStrengthReferenceStrength {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance: Option<CodeableConcept>,
    pub strength: Ratio,
    #[serde(rename = "strengthLowLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strength_low_limit: Option<Ratio>,
    #[serde(rename = "measurementPoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductInteractionInteractantItem {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductInteractionInteractant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemReference")]
    pub item_reference: Reference,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductInteraction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactant: Option<Vec<MedicinalProductInteractionInteractant>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incidence: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub management: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductManufactured {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "manufacturedDoseForm")]
    pub manufactured_dose_form: CodeableConcept,
    #[serde(rename = "unitOfPresentation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_presentation: Option<CodeableConcept>,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<Reference>>,
    #[serde(rename = "physicalCharacteristics")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[serde(rename = "otherCharacteristics")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_characteristics: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPackagedPackageItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<Vec<CodeableConcept>>,
    #[serde(rename = "alternateMaterial")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate_material: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Vec<Reference>>,
    #[serde(rename = "manufacturedItem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufactured_item: Option<Vec<Reference>>,
    #[serde(rename = "packageItem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_item: Option<Vec<MedicinalProductPackagedPackageItem>>,
    #[serde(rename = "physicalCharacteristics")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[serde(rename = "otherCharacteristics")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_characteristics: Option<Vec<CodeableConcept>>,
    #[serde(rename = "shelfLifeStorage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPackagedBatchIdentifier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "outerPackaging")]
    pub outer_packaging: Identifier,
    #[serde(rename = "immediatePackaging")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub immediate_packaging: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPackaged {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "legalStatusOfSupply")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[serde(rename = "marketingStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    #[serde(rename = "marketingAuthorization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketing_authorization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<Vec<Reference>>,
    #[serde(rename = "batchIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_identifier: Option<Vec<MedicinalProductPackagedBatchIdentifier>>,
    #[serde(rename = "packageItem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_item: Option<Vec<MedicinalProductPackagedPackageItem>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceuticalCharacteristics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "firstDose")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_dose: Option<Quantity>,
    #[serde(rename = "maxSingleDose")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_single_dose: Option<Quantity>,
    #[serde(rename = "maxDosePerDay")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dose_per_day: Option<Quantity>,
    #[serde(rename = "maxDosePerTreatmentPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dose_per_treatment_period: Option<Ratio>,
    #[serde(rename = "maxTreatmentPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_treatment_period: Option<Duration>,
    #[serde(rename = "targetSpecies")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_species: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpecies>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpecies {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "withdrawalPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub withdrawal_period: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpeciesWithdrawalPeriod>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceutical {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "administrableDoseForm")]
    pub administrable_dose_form: CodeableConcept,
    #[serde(rename = "unitOfPresentation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_presentation: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingredient: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristics: Option<Vec<MedicinalProductPharmaceuticalCharacteristics>>,
    #[serde(rename = "routeOfAdministration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_of_administration: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministration>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpeciesWithdrawalPeriod {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub tissue: CodeableConcept,
    pub value: Quantity,
    #[serde(rename = "supportingInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_information: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MedicinalProductUndesirableEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Vec<Reference>>,
    #[serde(rename = "symptomConditionEffect")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symptom_condition_effect: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<CodeableConcept>,
    #[serde(rename = "frequencyOfOccurrence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_of_occurrence: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<Vec<Population>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDefinitionAllowedResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub message: Canonical,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub situation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDefinitionFocus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Canonical>,
    pub min: UnsignedInt,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageDefinitionEvent {
    Coding(Coding),
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "eventCoding")]
    pub event_coding: Coding,
    #[serde(rename = "eventUri")]
    pub event_uri: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Vec<MessageDefinitionFocus>>,
    #[serde(rename = "responseRequired")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_required: Option<Code>,
    #[serde(rename = "allowedResponse")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_response: Option<Vec<MessageDefinitionAllowedResponse>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<Vec<Canonical>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageHeaderResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum MessageHeaderEvent {
    Coding(Coding),
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageHeader {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "eventCoding")]
    pub event_coding: Coding,
    #[serde(rename = "eventUri")]
    pub event_uri: Uri,
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
pub struct MessageHeaderSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct MolecularSequenceRepository {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "datasetId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dataset_id: Option<String>,
    #[serde(rename = "variantsetId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variantset_id: Option<String>,
    #[serde(rename = "readsetId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readset_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceQuality {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "standardSequence")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standard_sequence: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "truthTP")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truth_t_p: Option<Decimal>,
    #[serde(rename = "queryTP")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_t_p: Option<Decimal>,
    #[serde(rename = "truthFN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truth_f_n: Option<Decimal>,
    #[serde(rename = "queryFP")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_f_p: Option<Decimal>,
    #[serde(rename = "gtFP")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gt_f_p: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall: Option<Decimal>,
    #[serde(rename = "fScore")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "variantType")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct MolecularSequenceVariant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Integer>,
    #[serde(rename = "observedAllele")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_allele: Option<String>,
    #[serde(rename = "referenceAllele")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_allele: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cigar: Option<String>,
    #[serde(rename = "variantPointer")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_pointer: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceReferenceSeq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chromosome: Option<CodeableConcept>,
    #[serde(rename = "genomeBuild")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genome_build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Code>,
    #[serde(rename = "referenceSeqId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_seq_id: Option<CodeableConcept>,
    #[serde(rename = "referenceSeqPointer")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_seq_pointer: Option<Reference>,
    #[serde(rename = "referenceSeqString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_seq_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strand: Option<Code>,
    #[serde(rename = "windowStart")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_start: Option<Integer>,
    #[serde(rename = "windowEnd")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_end: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceStructureVariantOuter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequence {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "referenceSeq")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_seq: Option<MolecularSequenceReferenceSeq>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<Vec<MolecularSequenceVariant>>,
    #[serde(rename = "observedSeq")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_seq: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<Vec<MolecularSequenceQuality>>,
    #[serde(rename = "readCoverage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_coverage: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<Vec<MolecularSequenceRepository>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Vec<Reference>>,
    #[serde(rename = "structureVariant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure_variant: Option<Vec<MolecularSequenceStructureVariant>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MolecularSequenceQualityRoc {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<Vec<Integer>>,
    #[serde(rename = "numTP")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_t_p: Option<Vec<Integer>>,
    #[serde(rename = "numFP")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_f_p: Option<Vec<Integer>>,
    #[serde(rename = "numFN")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_f_n: Option<Vec<Integer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<Vec<Decimal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<Vec<Decimal>>,
    #[serde(rename = "fMeasure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub f_measure: Option<Vec<Decimal>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamingSystemUniqueId {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(rename = "uniqueId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_id: Option<Vec<NamingSystemUniqueId>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderOralDietNutrient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier: Option<CodeableConcept>,
    #[serde(rename = "foodType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub food_type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderSupplement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "productName")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct NutritionOrderEnteralFormula {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "baseFormulaType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_formula_type: Option<CodeableConcept>,
    #[serde(rename = "baseFormulaProductName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_formula_product_name: Option<String>,
    #[serde(rename = "additiveType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive_type: Option<CodeableConcept>,
    #[serde(rename = "additiveProductName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive_product_name: Option<String>,
    #[serde(rename = "caloricDensity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caloric_density: Option<Quantity>,
    #[serde(rename = "routeofAdministration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routeof_administration: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub administration: Option<Vec<NutritionOrderEnteralFormulaAdministration>>,
    #[serde(rename = "maxVolumeToDeliver")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_volume_to_deliver: Option<Quantity>,
    #[serde(rename = "administrationInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub administration_instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrder {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "allergyIntolerance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allergy_intolerance: Option<Vec<Reference>>,
    #[serde(rename = "foodPreferenceModifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub food_preference_modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "excludeFoodModifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_food_modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "oralDiet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oral_diet: Option<NutritionOrderOralDiet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplement: Option<Vec<NutritionOrderSupplement>>,
    #[serde(rename = "enteralFormula")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enteral_formula: Option<NutritionOrderEnteralFormula>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderOralDiet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Vec<Timing>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nutrient: Option<Vec<NutritionOrderOralDietNutrient>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub texture: Option<Vec<NutritionOrderOralDietTexture>>,
    #[serde(rename = "fluidConsistencyType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fluid_consistency_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NutritionOrderEnteralFormulaAdministrationRate {
    Quantity(Quantity),
    Ratio(Ratio),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NutritionOrderEnteralFormulaAdministration {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "rateQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_quantity: Option<Quantity>,
    #[serde(rename = "rateRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_ratio: Option<Ratio>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationEffective {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
    Instant(Instant),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationValue {
    Quantity(Quantity),
    CodeableConcept(CodeableConcept),
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Range(Range),
    Ratio(Ratio),
    SampledData(SampledData),
    Time(Time),
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "effectiveDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_date_time: Option<DateTime>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(rename = "effectiveTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_timing: Option<Timing>,
    #[serde(rename = "effectiveInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_instant: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<Instant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_ratio: Option<Ratio>,
    #[serde(rename = "valueSampledData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_sampled_data: Option<SampledData>,
    #[serde(rename = "valueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valuePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_period: Option<Period>,
    #[serde(rename = "dataAbsentReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_absent_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<Reference>,
    #[serde(rename = "referenceRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
    #[serde(rename = "hasMember")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_member: Option<Vec<Reference>>,
    #[serde(rename = "derivedFrom")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<Vec<ObservationComponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationComponentValue {
    Quantity(Quantity),
    CodeableConcept(CodeableConcept),
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Range(Range),
    Ratio(Ratio),
    SampledData(SampledData),
    Time(Time),
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_ratio: Option<Ratio>,
    #[serde(rename = "valueSampledData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_sampled_data: Option<SampledData>,
    #[serde(rename = "valueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valuePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_period: Option<Period>,
    #[serde(rename = "dataAbsentReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_absent_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpretation: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referenceRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReferenceRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<Quantity>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "appliesTo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "permittedDataType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permitted_data_type: Option<Vec<Code>>,
    #[serde(rename = "multipleResultsAllowed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple_results_allowed: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "preferredReportName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_report_name: Option<String>,
    #[serde(rename = "quantitativeDetails")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantitative_details: Option<ObservationDefinitionQuantitativeDetails>,
    #[serde(rename = "qualifiedInterval")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualified_interval: Option<Vec<ObservationDefinitionQualifiedInterval>>,
    #[serde(rename = "validCodedValueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_coded_value_set: Option<Reference>,
    #[serde(rename = "normalCodedValueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal_coded_value_set: Option<Reference>,
    #[serde(rename = "abnormalCodedValueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abnormal_coded_value_set: Option<Reference>,
    #[serde(rename = "criticalCodedValueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_coded_value_set: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationDefinitionQuantitativeDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "customaryUnit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customary_unit: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<CodeableConcept>,
    #[serde(rename = "conversionFactor")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversion_factor: Option<Decimal>,
    #[serde(rename = "decimalPrecision")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decimal_precision: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationDefinitionQualifiedInterval {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<CodeableConcept>,
    #[serde(rename = "appliesTo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age: Option<Range>,
    #[serde(rename = "gestationalAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gestational_age: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinitionOverload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "parameterName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_name: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(rename = "affectsState")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "inputProfile")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_profile: Option<Canonical>,
    #[serde(rename = "outputProfile")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_profile: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<OperationDefinitionParameter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overload: Option<Vec<OperationDefinitionOverload>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinitionParameterBinding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub strength: Code,
    #[serde(rename = "valueSet")]
    pub value_set: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinitionParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub min: Integer,
    pub max: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(rename = "targetProfile")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<Vec<Canonical>>,
    #[serde(rename = "searchType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<OperationDefinitionParameterBinding>,
    #[serde(rename = "referencedFrom")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referenced_from: Option<Vec<OperationDefinitionParameterReferencedFrom>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<Vec<OperationDefinitionParameter>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDefinitionParameterReferencedFrom {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: String,
    #[serde(rename = "sourceId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationOutcome {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct Organization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<OrganizationContact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrganizationContact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct OrganizationAffiliation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Reference>,
    #[serde(rename = "participatingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participating_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Vec<Reference>>,
    #[serde(rename = "healthcareService")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub healthcare_service: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<ParametersParameter>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParametersParameterValue {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParametersParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "valueBase64Binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_base64_binary: Option<Base64Binary>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_canonical: Option<Canonical>,
    #[serde(rename = "valueCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Code>,
    #[serde(rename = "valueDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valueDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_decimal: Option<Decimal>,
    #[serde(rename = "valueId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_id: Option<Id>,
    #[serde(rename = "valueInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_instant: Option<Instant>,
    #[serde(rename = "valueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueMarkdown")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_markdown: Option<Markdown>,
    #[serde(rename = "valueOid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_oid: Option<Oid>,
    #[serde(rename = "valuePositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_positive_int: Option<PositiveInt>,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "valueUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_uri: Option<Uri>,
    #[serde(rename = "valueUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_url: Option<Url>,
    #[serde(rename = "valueUuid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_uuid: Option<Uuid>,
    #[serde(rename = "valueAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_address: Option<Address>,
    #[serde(rename = "valueAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_age: Option<Age>,
    #[serde(rename = "valueAnnotation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_annotation: Option<Annotation>,
    #[serde(rename = "valueAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueCoding")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_coding: Option<Coding>,
    #[serde(rename = "valueContactPoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_contact_point: Option<ContactPoint>,
    #[serde(rename = "valueCount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_count: Option<Count>,
    #[serde(rename = "valueDistance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_distance: Option<Distance>,
    #[serde(rename = "valueDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_duration: Option<Duration>,
    #[serde(rename = "valueHumanName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_human_name: Option<HumanName>,
    #[serde(rename = "valueIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_identifier: Option<Identifier>,
    #[serde(rename = "valueMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_money: Option<Money>,
    #[serde(rename = "valuePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_period: Option<Period>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_ratio: Option<Ratio>,
    #[serde(rename = "valueReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_reference: Option<Reference>,
    #[serde(rename = "valueSampledData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_sampled_data: Option<SampledData>,
    #[serde(rename = "valueSignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_signature: Option<Signature>,
    #[serde(rename = "valueTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_timing: Option<Timing>,
    #[serde(rename = "valueContactDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_contact_detail: Option<ContactDetail>,
    #[serde(rename = "valueContributor")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_contributor: Option<Contributor>,
    #[serde(rename = "valueDataRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_data_requirement: Option<DataRequirement>,
    #[serde(rename = "valueExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_expression: Option<Expression>,
    #[serde(rename = "valueParameterDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "valueRelatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "valueTriggerDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "valueUsageContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_usage_context: Option<UsageContext>,
    #[serde(rename = "valueDosage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_dosage: Option<Dosage>,
    #[serde(rename = "valueMeta")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_meta: Option<Meta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Resource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<Vec<ParametersParameter>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatientContact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum PatientDeceased {
    Boolean(Boolean),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatientMultipleBirth {
    Boolean(Boolean),
    Integer(Integer),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "birthDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<Date>,
    #[serde(rename = "deceasedBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deceased_boolean: Option<Boolean>,
    #[serde(rename = "deceasedDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deceased_date_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(rename = "maritalStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marital_status: Option<CodeableConcept>,
    #[serde(rename = "multipleBirthBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple_birth_boolean: Option<Boolean>,
    #[serde(rename = "multipleBirthInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple_birth_integer: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<PatientContact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication: Option<Vec<PatientCommunication>>,
    #[serde(rename = "generalPractitioner")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub general_practitioner: Option<Vec<Reference>>,
    #[serde(rename = "managingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managing_organization: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<PatientLink>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatientCommunication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub other: Reference,
    #[serde(rename = "type")]
    pub r#type: Code,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentNotice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "paymentDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payee: Option<Reference>,
    pub recipient: Reference,
    pub amount: Money,
    #[serde(rename = "paymentStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_status: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentReconciliationDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentReconciliation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    pub created: DateTime,
    #[serde(rename = "paymentIssuer")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "paymentIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Vec<PaymentReconciliationDetail>>,
    #[serde(rename = "formCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_code: Option<CodeableConcept>,
    #[serde(rename = "processNote")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_note: Option<Vec<PaymentReconciliationProcessNote>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Person {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<HumanName>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<Code>,
    #[serde(rename = "birthDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<Date>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<Address>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<Attachment>,
    #[serde(rename = "managingOrganization")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assurance: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionActionCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionActionDynamicValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionTiming {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Duration(Duration),
    Range(Range),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionDefinition {
    Canonical(Canonical),
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "textEquivalent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_equivalent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "goalId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<Vec<Id>>,
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<Vec<TriggerDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<PlanDefinitionActionCondition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<DataRequirement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<DataRequirement>>,
    #[serde(rename = "relatedAction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_action: Option<Vec<PlanDefinitionActionRelatedAction>>,
    #[serde(rename = "timingDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_date_time: Option<DateTime>,
    #[serde(rename = "timingAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_age: Option<Age>,
    #[serde(rename = "timingPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_period: Option<Period>,
    #[serde(rename = "timingDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_duration: Option<Duration>,
    #[serde(rename = "timingRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_range: Option<Range>,
    #[serde(rename = "timingTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_timing: Option<Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<PlanDefinitionActionParticipant>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "groupingBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping_behavior: Option<Code>,
    #[serde(rename = "selectionBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_behavior: Option<Code>,
    #[serde(rename = "requiredBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_behavior: Option<Code>,
    #[serde(rename = "precheckBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precheck_behavior: Option<Code>,
    #[serde(rename = "cardinalityBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality_behavior: Option<Code>,
    #[serde(rename = "definitionCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_canonical: Option<Canonical>,
    #[serde(rename = "definitionUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_uri: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Canonical>,
    #[serde(rename = "dynamicValue")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_value: Option<Vec<PlanDefinitionActionDynamicValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<PlanDefinitionAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionGoal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionRelatedActionOffset {
    Duration(Duration),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionActionRelatedAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[serde(rename = "offsetDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_duration: Option<Duration>,
    #[serde(rename = "offsetRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_range: Option<Range>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<Vec<PlanDefinitionGoal>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<PlanDefinitionAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionGoalTargetDetail {
    Quantity(Quantity),
    Range(Range),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionGoalTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measure: Option<CodeableConcept>,
    #[serde(rename = "detailQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_quantity: Option<Quantity>,
    #[serde(rename = "detailRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_range: Option<Range>,
    #[serde(rename = "detailCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_codeable_concept: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDefinitionActionParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Practitioner {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "birthDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct PractitionerQualification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct PractitionerRole {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "healthcareService")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub healthcare_service: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(rename = "availableTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_time: Option<Vec<PractitionerRoleAvailableTime>>,
    #[serde(rename = "notAvailable")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_available: Option<Vec<PractitionerRoleNotAvailable>>,
    #[serde(rename = "availabilityExceptions")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub during: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PractitionerRoleAvailableTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "availableStartTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_start_time: Option<Time>,
    #[serde(rename = "availableEndTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_end_time: Option<Time>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedureFocalDevice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<CodeableConcept>,
    pub manipulated: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcedurePerformed {
    DateTime(DateTime),
    Period(Period),
    String(String),
    Age(Age),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Procedure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "performedDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performed_date_time: Option<DateTime>,
    #[serde(rename = "performedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performed_period: Option<Period>,
    #[serde(rename = "performedString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performed_string: Option<String>,
    #[serde(rename = "performedAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performed_age: Option<Age>,
    #[serde(rename = "performedRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performed_range: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorder: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asserter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<ProcedurePerformer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complication: Option<Vec<CodeableConcept>>,
    #[serde(rename = "complicationDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complication_detail: Option<Vec<Reference>>,
    #[serde(rename = "followUp")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_up: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "focalDevice")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focal_device: Option<Vec<ProcedureFocalDevice>>,
    #[serde(rename = "usedReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_reference: Option<Vec<Reference>>,
    #[serde(rename = "usedCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedurePerformer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(rename = "onBehalfOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceEntity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Code,
    pub what: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Vec<ProvenanceAgent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceAgent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Reference,
    #[serde(rename = "onBehalfOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProvenanceOccurred {
    Period(Period),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<Reference>>,
    #[serde(rename = "occurredPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_period: Option<Period>,
    #[serde(rename = "occurredDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurred_date_time: Option<DateTime>,
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
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemEnableWhenAnswer {
    Boolean(Boolean),
    Decimal(Decimal),
    Integer(Integer),
    Date(Date),
    DateTime(DateTime),
    Time(Time),
    String(String),
    Coding(Coding),
    Quantity(Quantity),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireItemEnableWhen {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub question: String,
    pub operator: Code,
    #[serde(rename = "answerBoolean")]
    pub answer_boolean: Boolean,
    #[serde(rename = "answerDecimal")]
    pub answer_decimal: Decimal,
    #[serde(rename = "answerInteger")]
    pub answer_integer: Integer,
    #[serde(rename = "answerDate")]
    pub answer_date: Date,
    #[serde(rename = "answerDateTime")]
    pub answer_date_time: DateTime,
    #[serde(rename = "answerTime")]
    pub answer_time: Time,
    #[serde(rename = "answerString")]
    pub answer_string: String,
    #[serde(rename = "answerCoding")]
    pub answer_coding: Coding,
    #[serde(rename = "answerQuantity")]
    pub answer_quantity: Quantity,
    #[serde(rename = "answerReference")]
    pub answer_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemAnswerOptionValue {
    Integer(Integer),
    Date(Date),
    Time(Time),
    String(String),
    Coding(Coding),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireItemAnswerOption {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    #[serde(rename = "initialSelected")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_selected: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Questionnaire {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "derivedFrom")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<Vec<Canonical>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_type: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<Coding>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<QuestionnaireItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "enableWhen")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_when: Option<Vec<QuestionnaireItemEnableWhen>>,
    #[serde(rename = "enableBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_behavior: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeats: Option<Boolean>,
    #[serde(rename = "readOnly")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<Boolean>,
    #[serde(rename = "maxLength")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<Integer>,
    #[serde(rename = "answerValueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_value_set: Option<Canonical>,
    #[serde(rename = "answerOption")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer_option: Option<Vec<QuestionnaireItemAnswerOption>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial: Option<Vec<QuestionnaireItemInitial>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<QuestionnaireItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemInitialValue {
    Boolean(Boolean),
    Decimal(Decimal),
    Integer(Integer),
    Date(Date),
    DateTime(DateTime),
    Time(Time),
    String(String),
    Uri(Uri),
    Attachment(Attachment),
    Coding(Coding),
    Quantity(Quantity),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireItemInitial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueUri")]
    pub value_uri: Uri,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireResponseItemAnswerValue {
    Boolean(Boolean),
    Decimal(Decimal),
    Integer(Integer),
    Date(Date),
    DateTime(DateTime),
    Time(Time),
    String(String),
    Uri(Uri),
    Attachment(Attachment),
    Coding(Coding),
    Quantity(Quantity),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireResponseItemAnswer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_decimal: Option<Decimal>,
    #[serde(rename = "valueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_uri: Option<Uri>,
    #[serde(rename = "valueAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueCoding")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_coding: Option<Coding>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionnaireResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelatedPerson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "birthDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct RelatedPersonCommunication {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupActionRelatedActionOffset {
    Duration(Duration),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGroupActionRelatedAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[serde(rename = "offsetDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_duration: Option<Duration>,
    #[serde(rename = "offsetRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_range: Option<Range>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupActionTiming {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Duration(Duration),
    Range(Range),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGroupAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "textEquivalent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_equivalent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<RequestGroupActionCondition>>,
    #[serde(rename = "relatedAction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_action: Option<Vec<RequestGroupActionRelatedAction>>,
    #[serde(rename = "timingDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_date_time: Option<DateTime>,
    #[serde(rename = "timingAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_age: Option<Age>,
    #[serde(rename = "timingPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_period: Option<Period>,
    #[serde(rename = "timingDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_duration: Option<Duration>,
    #[serde(rename = "timingRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_range: Option<Range>,
    #[serde(rename = "timingTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_timing: Option<Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant: Option<Vec<Reference>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "groupingBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouping_behavior: Option<Code>,
    #[serde(rename = "selectionBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_behavior: Option<Code>,
    #[serde(rename = "requiredBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_behavior: Option<Code>,
    #[serde(rename = "precheckBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precheck_behavior: Option<Code>,
    #[serde(rename = "cardinalityBehavior")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality_behavior: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<RequestGroupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "authoredOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<RequestGroupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGroupActionCondition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<Expression>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "shortTitle")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    pub population: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure: Option<Reference>,
    #[serde(rename = "exposureAlternative")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure_alternative: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchElementDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "shortTitle")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<Vec<Canonical>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "variableType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub characteristic: Option<Vec<ResearchElementDefinitionCharacteristic>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicDefinition {
    CodeableConcept(CodeableConcept),
    Canonical(Canonical),
    Expression(Expression),
    DataRequirement(DataRequirement),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicStudyEffective {
    DateTime(DateTime),
    Period(Period),
    Duration(Duration),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicParticipantEffective {
    DateTime(DateTime),
    Period(Period),
    Duration(Duration),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchElementDefinitionCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "definitionCodeableConcept")]
    pub definition_codeable_concept: CodeableConcept,
    #[serde(rename = "definitionCanonical")]
    pub definition_canonical: Canonical,
    #[serde(rename = "definitionExpression")]
    pub definition_expression: Expression,
    #[serde(rename = "definitionDataRequirement")]
    pub definition_data_requirement: DataRequirement,
    #[serde(rename = "usageContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Boolean>,
    #[serde(rename = "unitOfMeasure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "studyEffectiveDescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_effective_description: Option<String>,
    #[serde(rename = "studyEffectiveDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_effective_date_time: Option<DateTime>,
    #[serde(rename = "studyEffectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_effective_period: Option<Period>,
    #[serde(rename = "studyEffectiveDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_effective_duration: Option<Duration>,
    #[serde(rename = "studyEffectiveTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_effective_timing: Option<Timing>,
    #[serde(rename = "studyEffectiveTimeFromStart")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_effective_time_from_start: Option<Duration>,
    #[serde(rename = "studyEffectiveGroupMeasure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_effective_group_measure: Option<Code>,
    #[serde(rename = "participantEffectiveDescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_description: Option<String>,
    #[serde(rename = "participantEffectiveDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_date_time: Option<DateTime>,
    #[serde(rename = "participantEffectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_period: Option<Period>,
    #[serde(rename = "participantEffectiveDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_duration: Option<Duration>,
    #[serde(rename = "participantEffectiveTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_timing: Option<Timing>,
    #[serde(rename = "participantEffectiveTimeFromStart")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_time_from_start: Option<Duration>,
    #[serde(rename = "participantEffectiveGroupMeasure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub participant_effective_group_measure: Option<Code>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchStudyObjective {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchStudyArm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "primaryPurposeType")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "principalInvestigator")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_investigator: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<Vec<Reference>>,
    #[serde(rename = "reasonStopped")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ResearchSubject {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    pub study: Reference,
    pub individual: Reference,
    #[serde(rename = "assignedArm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_arm: Option<String>,
    #[serde(rename = "actualArm")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_arm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consent: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentOccurrence {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskAssessment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentPredictionProbability {
    Decimal(Decimal),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentPredictionWhen {
    Period(Period),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskAssessmentPrediction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<CodeableConcept>,
    #[serde(rename = "probabilityDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probability_decimal: Option<Decimal>,
    #[serde(rename = "probabilityRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probability_range: Option<Range>,
    #[serde(rename = "qualitativeRisk")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualitative_risk: Option<CodeableConcept>,
    #[serde(rename = "relativeRisk")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_risk: Option<Decimal>,
    #[serde(rename = "whenPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_period: Option<Period>,
    #[serde(rename = "whenRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_range: Option<Range>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisRiskEstimatePrecisionEstimate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct RiskEvidenceSynthesisRiskEstimate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Decimal>,
    #[serde(rename = "unitOfMeasure")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "denominatorCount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denominator_count: Option<Integer>,
    #[serde(rename = "numeratorCount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numerator_count: Option<Integer>,
    #[serde(rename = "precisionEstimate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision_estimate: Option<Vec<RiskEvidenceSynthesisRiskEstimatePrecisionEstimate>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisCertainty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "certaintySubcomponent")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certainty_subcomponent: Option<Vec<RiskEvidenceSynthesisCertaintyCertaintySubcomponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "relatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "synthesisType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesis_type: Option<CodeableConcept>,
    #[serde(rename = "studyType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub study_type: Option<CodeableConcept>,
    pub population: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure: Option<Reference>,
    pub outcome: Reference,
    #[serde(rename = "sampleSize")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<RiskEvidenceSynthesisSampleSize>,
    #[serde(rename = "riskEstimate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_estimate: Option<RiskEvidenceSynthesisRiskEstimate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certainty: Option<Vec<RiskEvidenceSynthesisCertainty>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisCertaintyCertaintySubcomponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskEvidenceSynthesisSampleSize {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "numberOfStudies")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_studies: Option<Integer>,
    #[serde(rename = "numberOfParticipants")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_participants: Option<Integer>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Boolean>,
    #[serde(rename = "serviceCategory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<Vec<Reference>>,
    #[serde(rename = "planningHorizon")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planning_horizon: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchParameterComponent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub definition: Canonical,
    pub expression: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    #[serde(rename = "derivedFrom")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "xpathUsage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xpath_usage: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<Code>>,
    #[serde(rename = "multipleOr")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiple_or: Option<Boolean>,
    #[serde(rename = "multipleAnd")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestQuantity {
    Quantity(Quantity),
    Ratio(Ratio),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestAsNeeded {
    Boolean(Boolean),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "doNotPerform")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub do_not_perform: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "orderDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_detail: Option<Vec<CodeableConcept>>,
    #[serde(rename = "quantityQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity_quantity: Option<Quantity>,
    #[serde(rename = "quantityRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity_ratio: Option<Ratio>,
    #[serde(rename = "quantityRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity_range: Option<Range>,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_timing: Option<Timing>,
    #[serde(rename = "asNeededBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_needed_boolean: Option<Boolean>,
    #[serde(rename = "asNeededCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_needed_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "authoredOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(rename = "performerType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "locationCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "locationReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_reference: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "patientInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient_instruction: Option<String>,
    #[serde(rename = "relevantHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "serviceCategory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum SpecimenProcessingTime {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenProcessing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Vec<Reference>>,
    #[serde(rename = "timeDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_date_time: Option<DateTime>,
    #[serde(rename = "timePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Specimen {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "accessionIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accession_identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Reference>,
    #[serde(rename = "receivedTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectionCollected {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectionFastingStatus {
    CodeableConcept(CodeableConcept),
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenCollection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector: Option<Reference>,
    #[serde(rename = "collectedDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected_date_time: Option<DateTime>,
    #[serde(rename = "collectedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "bodySite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "fastingStatusCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fasting_status_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "fastingStatusDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fasting_status_duration: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenContainerAdditive {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenContainer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<Quantity>,
    #[serde(rename = "specimenQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specimen_quantity: Option<Quantity>,
    #[serde(rename = "additiveCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "additiveReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive_reference: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionTypeTestedContainerAdditiveAdditive {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinitionTypeTestedContainerAdditive {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "additiveCodeableConcept")]
    pub additive_codeable_concept: CodeableConcept,
    #[serde(rename = "additiveReference")]
    pub additive_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionTypeTestedContainerMinimumVolume {
    Quantity(Quantity),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinitionTypeTestedContainer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<CodeableConcept>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cap: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capacity: Option<Quantity>,
    #[serde(rename = "minimumVolumeQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_volume_quantity: Option<Quantity>,
    #[serde(rename = "minimumVolumeString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_volume_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additive: Option<Vec<SpecimenDefinitionTypeTestedContainerAdditive>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preparation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinitionTypeTestedHandling {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "temperatureQualifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_qualifier: Option<CodeableConcept>,
    #[serde(rename = "temperatureRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_range: Option<Range>,
    #[serde(rename = "maxDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "typeCollected")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_collected: Option<CodeableConcept>,
    #[serde(rename = "patientPreparation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient_preparation: Option<Vec<CodeableConcept>>,
    #[serde(rename = "timeAspect")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_aspect: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collection: Option<Vec<CodeableConcept>>,
    #[serde(rename = "typeTested")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_tested: Option<Vec<SpecimenDefinitionTypeTested>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecimenDefinitionTypeTested {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "isDerived")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_derived: Option<Boolean>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    pub preference: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<SpecimenDefinitionTypeTestedContainer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(rename = "retentionTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_time: Option<Duration>,
    #[serde(rename = "rejectionCriterion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_criterion: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handling: Option<Vec<SpecimenDefinitionTypeTestedHandling>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureDefinitionSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct StructureDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<Vec<Coding>>,
    #[serde(rename = "fhirVersion")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fhir_version: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<Vec<StructureDefinitionMapping>>,
    pub kind: Code,
    #[serde(rename = "abstract")]
    pub r#abstract: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<StructureDefinitionContext>>,
    #[serde(rename = "contextInvariant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_invariant: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub r#type: Uri,
    #[serde(rename = "baseDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<Vec<ElementDefinition>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureDefinitionContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub expression: String,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct StructureMapGroupRuleTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Id>,
    #[serde(rename = "contextType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<Id>,
    #[serde(rename = "listMode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_mode: Option<Vec<Code>>,
    #[serde(rename = "listRuleId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_rule_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<StructureMapGroupRuleTargetParameter>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapGroupRuleSourceDefaultValue {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRuleSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(rename = "defaultValueBase64Binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_base64_binary: Option<Base64Binary>,
    #[serde(rename = "defaultValueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_boolean: Option<Boolean>,
    #[serde(rename = "defaultValueCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_canonical: Option<Canonical>,
    #[serde(rename = "defaultValueCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_code: Option<Code>,
    #[serde(rename = "defaultValueDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_date: Option<Date>,
    #[serde(rename = "defaultValueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_date_time: Option<DateTime>,
    #[serde(rename = "defaultValueDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_decimal: Option<Decimal>,
    #[serde(rename = "defaultValueId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_id: Option<Id>,
    #[serde(rename = "defaultValueInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_instant: Option<Instant>,
    #[serde(rename = "defaultValueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_integer: Option<Integer>,
    #[serde(rename = "defaultValueMarkdown")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_markdown: Option<Markdown>,
    #[serde(rename = "defaultValueOid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_oid: Option<Oid>,
    #[serde(rename = "defaultValuePositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_positive_int: Option<PositiveInt>,
    #[serde(rename = "defaultValueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_string: Option<String>,
    #[serde(rename = "defaultValueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_time: Option<Time>,
    #[serde(rename = "defaultValueUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "defaultValueUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_uri: Option<Uri>,
    #[serde(rename = "defaultValueUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_url: Option<Url>,
    #[serde(rename = "defaultValueUuid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_uuid: Option<Uuid>,
    #[serde(rename = "defaultValueAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_address: Option<Address>,
    #[serde(rename = "defaultValueAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_age: Option<Age>,
    #[serde(rename = "defaultValueAnnotation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_annotation: Option<Annotation>,
    #[serde(rename = "defaultValueAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_attachment: Option<Attachment>,
    #[serde(rename = "defaultValueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "defaultValueCoding")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_coding: Option<Coding>,
    #[serde(rename = "defaultValueContactPoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_contact_point: Option<ContactPoint>,
    #[serde(rename = "defaultValueCount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_count: Option<Count>,
    #[serde(rename = "defaultValueDistance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_distance: Option<Distance>,
    #[serde(rename = "defaultValueDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_duration: Option<Duration>,
    #[serde(rename = "defaultValueHumanName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_human_name: Option<HumanName>,
    #[serde(rename = "defaultValueIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_identifier: Option<Identifier>,
    #[serde(rename = "defaultValueMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_money: Option<Money>,
    #[serde(rename = "defaultValuePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_period: Option<Period>,
    #[serde(rename = "defaultValueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_quantity: Option<Quantity>,
    #[serde(rename = "defaultValueRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_range: Option<Range>,
    #[serde(rename = "defaultValueRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_ratio: Option<Ratio>,
    #[serde(rename = "defaultValueReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_reference: Option<Reference>,
    #[serde(rename = "defaultValueSampledData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_sampled_data: Option<SampledData>,
    #[serde(rename = "defaultValueSignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_signature: Option<Signature>,
    #[serde(rename = "defaultValueTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_timing: Option<Timing>,
    #[serde(rename = "defaultValueContactDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_contact_detail: Option<ContactDetail>,
    #[serde(rename = "defaultValueContributor")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_contributor: Option<Contributor>,
    #[serde(rename = "defaultValueDataRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_data_requirement: Option<DataRequirement>,
    #[serde(rename = "defaultValueExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_expression: Option<Expression>,
    #[serde(rename = "defaultValueParameterDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "defaultValueRelatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "defaultValueTriggerDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "defaultValueUsageContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_usage_context: Option<UsageContext>,
    #[serde(rename = "defaultValueDosage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_dosage: Option<Dosage>,
    #[serde(rename = "defaultValueMeta")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_meta: Option<Meta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    #[serde(rename = "listMode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_mode: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variable: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    #[serde(rename = "logMessage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRuleDependent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct StructureMapGroupInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapGroupRuleTargetParameterValue {
    Id(Id),
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Decimal(Decimal),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapGroupRuleTargetParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueId")]
    pub value_id: Id,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureMapStructure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Canonical,
    pub mode: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct SubscriptionChannel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct SubstanceInstance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceIngredientSubstance {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceIngredient {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Ratio>,
    #[serde(rename = "substanceCodeableConcept")]
    pub substance_codeable_concept: CodeableConcept,
    #[serde(rename = "substanceReference")]
    pub substance_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Substance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct SubstanceNucleicAcidSubunitSugar {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "residueSite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub residue_site: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceNucleicAcidSubunitLinkage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "residueSite")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub residue_site: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceNucleicAcid {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "sequenceType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_type: Option<CodeableConcept>,
    #[serde(rename = "numberOfSubunits")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_subunits: Option<Integer>,
    #[serde(rename = "areaOfHybridisation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_of_hybridisation: Option<String>,
    #[serde(rename = "oligoNucleotideType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oligo_nucleotide_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subunit: Option<Vec<SubstanceNucleicAcidSubunit>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceNucleicAcidSubunit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subunit: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Integer>,
    #[serde(rename = "sequenceAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_attachment: Option<Attachment>,
    #[serde(rename = "fivePrime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_prime: Option<CodeableConcept>,
    #[serde(rename = "threePrime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub three_prime: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linkage: Option<Vec<SubstanceNucleicAcidSubunitLinkage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sugar: Option<Vec<SubstanceNucleicAcidSubunitSugar>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry: Option<CodeableConcept>,
    #[serde(rename = "copolymerConnectivity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copolymer_connectivity: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modification: Option<Vec<String>>,
    #[serde(rename = "monomerSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monomer_set: Option<Vec<SubstancePolymerMonomerSet>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Vec<SubstancePolymerRepeat>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerRepeat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "numberOfUnits")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_units: Option<Integer>,
    #[serde(rename = "averageMolecularFormula")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_molecular_formula: Option<String>,
    #[serde(rename = "repeatUnitAmountType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_unit_amount_type: Option<CodeableConcept>,
    #[serde(rename = "repeatUnit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_unit: Option<Vec<SubstancePolymerRepeatRepeatUnit>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerRepeatRepeatUnit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "orientationOfPolymerisation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation_of_polymerisation: Option<CodeableConcept>,
    #[serde(rename = "repeatUnit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<SubstanceAmount>,
    #[serde(rename = "degreeOfPolymerisation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degree_of_polymerisation: Option<Vec<SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation>>,
    #[serde(rename = "structuralRepresentation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structural_representation: Option<Vec<SubstancePolymerRepeatRepeatUnitStructuralRepresentation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degree: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<SubstanceAmount>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerMonomerSetStartingMaterial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<CodeableConcept>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "isDefining")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstancePolymerMonomerSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "ratioType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio_type: Option<CodeableConcept>,
    #[serde(rename = "startingMaterial")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starting_material: Option<Vec<SubstancePolymerMonomerSetStartingMaterial>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceProtein {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "sequenceType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_type: Option<CodeableConcept>,
    #[serde(rename = "numberOfSubunits")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_of_subunits: Option<Integer>,
    #[serde(rename = "disulfideLinkage")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subunit: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<Integer>,
    #[serde(rename = "sequenceAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence_attachment: Option<Attachment>,
    #[serde(rename = "nTerminalModificationId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_terminal_modification_id: Option<Identifier>,
    #[serde(rename = "nTerminalModification")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n_terminal_modification: Option<String>,
    #[serde(rename = "cTerminalModificationId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c_terminal_modification_id: Option<Identifier>,
    #[serde(rename = "cTerminalModification")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub c_terminal_modification: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceReferenceInformationTargetAmount {
    Quantity(Quantity),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceReferenceInformationTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Identifier>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism: Option<CodeableConcept>,
    #[serde(rename = "organismType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism_type: Option<CodeableConcept>,
    #[serde(rename = "amountQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_quantity: Option<Quantity>,
    #[serde(rename = "amountRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_range: Option<Range>,
    #[serde(rename = "amountString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_string: Option<String>,
    #[serde(rename = "amountType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gene: Option<Vec<SubstanceReferenceInformationGene>>,
    #[serde(rename = "geneElement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gene_element: Option<Vec<SubstanceReferenceInformationGeneElement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<Vec<SubstanceReferenceInformationClassification>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<SubstanceReferenceInformationTarget>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceReferenceInformationGeneElement {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct SubstanceReferenceInformationGene {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "geneSequenceOrigin")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gene_sequence_origin: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gene: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialOrganismOrganismGeneral {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct SubstanceSourceMaterialPartDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<CodeableConcept>,
    #[serde(rename = "partLocation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_location: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "sourceMaterialClass")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_material_class: Option<CodeableConcept>,
    #[serde(rename = "sourceMaterialType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_material_type: Option<CodeableConcept>,
    #[serde(rename = "sourceMaterialState")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_material_state: Option<CodeableConcept>,
    #[serde(rename = "organismId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism_id: Option<Identifier>,
    #[serde(rename = "organismName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism_name: Option<String>,
    #[serde(rename = "parentSubstanceId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_substance_id: Option<Vec<Identifier>>,
    #[serde(rename = "parentSubstanceName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_substance_name: Option<Vec<String>>,
    #[serde(rename = "countryOfOrigin")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_of_origin: Option<Vec<CodeableConcept>>,
    #[serde(rename = "geographicalLocation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geographical_location: Option<Vec<String>>,
    #[serde(rename = "developmentStage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub development_stage: Option<CodeableConcept>,
    #[serde(rename = "fractionDescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fraction_description: Option<Vec<SubstanceSourceMaterialFractionDescription>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism: Option<SubstanceSourceMaterialOrganism>,
    #[serde(rename = "partDescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_description: Option<Vec<SubstanceSourceMaterialPartDescription>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialFractionDescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fraction: Option<String>,
    #[serde(rename = "materialType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialOrganism {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genus: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub species: Option<CodeableConcept>,
    #[serde(rename = "intraspecificType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intraspecific_type: Option<CodeableConcept>,
    #[serde(rename = "intraspecificDescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intraspecific_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<Vec<SubstanceSourceMaterialOrganismAuthor>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hybrid: Option<SubstanceSourceMaterialOrganismHybrid>,
    #[serde(rename = "organismGeneral")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organism_general: Option<SubstanceSourceMaterialOrganismOrganismGeneral>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialOrganismHybrid {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "maternalOrganismId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maternal_organism_id: Option<String>,
    #[serde(rename = "maternalOrganismName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maternal_organism_name: Option<String>,
    #[serde(rename = "paternalOrganismId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paternal_organism_id: Option<String>,
    #[serde(rename = "paternalOrganismName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paternal_organism_name: Option<String>,
    #[serde(rename = "hybridType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hybrid_type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSourceMaterialOrganismAuthor {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "authorType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_type: Option<CodeableConcept>,
    #[serde(rename = "authorDescription")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_description: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationStructure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereochemistry: Option<CodeableConcept>,
    #[serde(rename = "opticalActivity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optical_activity: Option<CodeableConcept>,
    #[serde(rename = "molecularFormula")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub molecular_formula: Option<String>,
    #[serde(rename = "molecularFormulaByMoiety")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub molecular_formula_by_moiety: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isotope: Option<Vec<SubstanceSpecificationStructureIsotope>>,
    #[serde(rename = "molecularWeight")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub molecular_weight: Option<SubstanceSpecificationStructureIsotopeMolecularWeight>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<Vec<SubstanceSpecificationStructureRepresentation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationStructureIsotopeMolecularWeight {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationStructureIsotope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substitution: Option<CodeableConcept>,
    #[serde(rename = "halfLife")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub half_life: Option<Quantity>,
    #[serde(rename = "molecularWeight")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub molecular_weight: Option<SubstanceSpecificationStructureIsotopeMolecularWeight>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationPropertyDefiningSubstance {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationPropertyAmount {
    Quantity(Quantity),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationProperty {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<String>,
    #[serde(rename = "definingSubstanceReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defining_substance_reference: Option<Reference>,
    #[serde(rename = "definingSubstanceCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defining_substance_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "amountQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_quantity: Option<Quantity>,
    #[serde(rename = "amountString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_string: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "referenceInformation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_information: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<SubstanceSpecificationStructure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<SubstanceSpecificationCode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Vec<SubstanceSpecificationName>>,
    #[serde(rename = "molecularWeight")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub molecular_weight: Option<Vec<SubstanceSpecificationStructureIsotopeMolecularWeight>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Vec<SubstanceSpecificationRelationship>>,
    #[serde(rename = "nucleicAcid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nucleic_acid: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polymer: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protein: Option<Reference>,
    #[serde(rename = "sourceMaterial")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_material: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationStructureRepresentation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationRelationshipSubstance {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationRelationshipAmount {
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationRelationship {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "substanceReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance_reference: Option<Reference>,
    #[serde(rename = "substanceCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub substance_codeable_concept: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<CodeableConcept>,
    #[serde(rename = "isDefining")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_defining: Option<Boolean>,
    #[serde(rename = "amountQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_quantity: Option<Quantity>,
    #[serde(rename = "amountRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_range: Option<Range>,
    #[serde(rename = "amountRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_ratio: Option<Ratio>,
    #[serde(rename = "amountString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_string: Option<String>,
    #[serde(rename = "amountRatioLowLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_ratio_low_limit: Option<Ratio>,
    #[serde(rename = "amountType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationMoietyAmount {
    Quantity(Quantity),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationMoiety {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereochemistry: Option<CodeableConcept>,
    #[serde(rename = "opticalActivity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optical_activity: Option<CodeableConcept>,
    #[serde(rename = "molecularFormula")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub molecular_formula: Option<String>,
    #[serde(rename = "amountQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_quantity: Option<Quantity>,
    #[serde(rename = "amountString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_string: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationCode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(rename = "statusDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceSpecificationName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct SubstanceSpecificationNameOfficial {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliverySuppliedItemItem {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyDeliverySuppliedItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Quantity>,
    #[serde(rename = "itemCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "itemReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliveryOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyDelivery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient: Option<Reference>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "suppliedItem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplied_item: Option<SupplyDeliverySuppliedItem>,
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_timing: Option<Timing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplier: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestItem {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<Code>,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: CodeableConcept,
    #[serde(rename = "itemReference")]
    pub item_reference: Reference,
    pub quantity: Quantity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<SupplyRequestParameter>>,
    #[serde(rename = "occurrenceDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occurrence_timing: Option<Timing>,
    #[serde(rename = "authoredOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supplier: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "deliverFrom")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deliver_from: Option<Reference>,
    #[serde(rename = "deliverTo")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deliver_to: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestParameterValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Range(Range),
    Boolean(Boolean),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupplyRequestParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
    #[serde(rename = "valueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskOutputValue {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Base64Binary,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueCanonical")]
    pub value_canonical: Canonical,
    #[serde(rename = "valueCode")]
    pub value_code: Code,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
    #[serde(rename = "valueId")]
    pub value_id: Id,
    #[serde(rename = "valueInstant")]
    pub value_instant: Instant,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueMarkdown")]
    pub value_markdown: Markdown,
    #[serde(rename = "valueOid")]
    pub value_oid: Oid,
    #[serde(rename = "valuePositiveInt")]
    pub value_positive_int: PositiveInt,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueUnsignedInt")]
    pub value_unsigned_int: UnsignedInt,
    #[serde(rename = "valueUri")]
    pub value_uri: Uri,
    #[serde(rename = "valueUrl")]
    pub value_url: Url,
    #[serde(rename = "valueUuid")]
    pub value_uuid: Uuid,
    #[serde(rename = "valueAddress")]
    pub value_address: Address,
    #[serde(rename = "valueAge")]
    pub value_age: Age,
    #[serde(rename = "valueAnnotation")]
    pub value_annotation: Annotation,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueContactPoint")]
    pub value_contact_point: ContactPoint,
    #[serde(rename = "valueCount")]
    pub value_count: Count,
    #[serde(rename = "valueDistance")]
    pub value_distance: Distance,
    #[serde(rename = "valueDuration")]
    pub value_duration: Duration,
    #[serde(rename = "valueHumanName")]
    pub value_human_name: HumanName,
    #[serde(rename = "valueIdentifier")]
    pub value_identifier: Identifier,
    #[serde(rename = "valueMoney")]
    pub value_money: Money,
    #[serde(rename = "valuePeriod")]
    pub value_period: Period,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    #[serde(rename = "valueRatio")]
    pub value_ratio: Ratio,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    #[serde(rename = "valueSampledData")]
    pub value_sampled_data: SampledData,
    #[serde(rename = "valueSignature")]
    pub value_signature: Signature,
    #[serde(rename = "valueTiming")]
    pub value_timing: Timing,
    #[serde(rename = "valueContactDetail")]
    pub value_contact_detail: ContactDetail,
    #[serde(rename = "valueContributor")]
    pub value_contributor: Contributor,
    #[serde(rename = "valueDataRequirement")]
    pub value_data_requirement: DataRequirement,
    #[serde(rename = "valueExpression")]
    pub value_expression: Expression,
    #[serde(rename = "valueParameterDefinition")]
    pub value_parameter_definition: ParameterDefinition,
    #[serde(rename = "valueRelatedArtifact")]
    pub value_related_artifact: RelatedArtifact,
    #[serde(rename = "valueTriggerDefinition")]
    pub value_trigger_definition: TriggerDefinition,
    #[serde(rename = "valueUsageContext")]
    pub value_usage_context: UsageContext,
    #[serde(rename = "valueDosage")]
    pub value_dosage: Dosage,
    #[serde(rename = "valueMeta")]
    pub value_meta: Meta,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRestriction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_canonical: Option<Canonical>,
    #[serde(rename = "instantiatesUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instantiates_uri: Option<Uri>,
    #[serde(rename = "basedOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_identifier: Option<Identifier>,
    #[serde(rename = "partOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "businessStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "for")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#for: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(rename = "executionPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_period: Option<Period>,
    #[serde(rename = "authoredOn")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authored_on: Option<DateTime>,
    #[serde(rename = "lastModified")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester: Option<Reference>,
    #[serde(rename = "performerType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performer_type: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<CodeableConcept>,
    #[serde(rename = "reasonReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_reference: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insurance: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "relevantHistory")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant_history: Option<Vec<Reference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restriction: Option<TaskRestriction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<TaskInput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Vec<TaskOutput>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskInputValue {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Base64Binary,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueCanonical")]
    pub value_canonical: Canonical,
    #[serde(rename = "valueCode")]
    pub value_code: Code,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
    #[serde(rename = "valueId")]
    pub value_id: Id,
    #[serde(rename = "valueInstant")]
    pub value_instant: Instant,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueMarkdown")]
    pub value_markdown: Markdown,
    #[serde(rename = "valueOid")]
    pub value_oid: Oid,
    #[serde(rename = "valuePositiveInt")]
    pub value_positive_int: PositiveInt,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueUnsignedInt")]
    pub value_unsigned_int: UnsignedInt,
    #[serde(rename = "valueUri")]
    pub value_uri: Uri,
    #[serde(rename = "valueUrl")]
    pub value_url: Url,
    #[serde(rename = "valueUuid")]
    pub value_uuid: Uuid,
    #[serde(rename = "valueAddress")]
    pub value_address: Address,
    #[serde(rename = "valueAge")]
    pub value_age: Age,
    #[serde(rename = "valueAnnotation")]
    pub value_annotation: Annotation,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueContactPoint")]
    pub value_contact_point: ContactPoint,
    #[serde(rename = "valueCount")]
    pub value_count: Count,
    #[serde(rename = "valueDistance")]
    pub value_distance: Distance,
    #[serde(rename = "valueDuration")]
    pub value_duration: Duration,
    #[serde(rename = "valueHumanName")]
    pub value_human_name: HumanName,
    #[serde(rename = "valueIdentifier")]
    pub value_identifier: Identifier,
    #[serde(rename = "valueMoney")]
    pub value_money: Money,
    #[serde(rename = "valuePeriod")]
    pub value_period: Period,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    #[serde(rename = "valueRatio")]
    pub value_ratio: Ratio,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    #[serde(rename = "valueSampledData")]
    pub value_sampled_data: SampledData,
    #[serde(rename = "valueSignature")]
    pub value_signature: Signature,
    #[serde(rename = "valueTiming")]
    pub value_timing: Timing,
    #[serde(rename = "valueContactDetail")]
    pub value_contact_detail: ContactDetail,
    #[serde(rename = "valueContributor")]
    pub value_contributor: Contributor,
    #[serde(rename = "valueDataRequirement")]
    pub value_data_requirement: DataRequirement,
    #[serde(rename = "valueExpression")]
    pub value_expression: Expression,
    #[serde(rename = "valueParameterDefinition")]
    pub value_parameter_definition: ParameterDefinition,
    #[serde(rename = "valueRelatedArtifact")]
    pub value_related_artifact: RelatedArtifact,
    #[serde(rename = "valueTriggerDefinition")]
    pub value_trigger_definition: TriggerDefinition,
    #[serde(rename = "valueUsageContext")]
    pub value_usage_context: UsageContext,
    #[serde(rename = "valueDosage")]
    pub value_dosage: Dosage,
    #[serde(rename = "valueMeta")]
    pub value_meta: Meta,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesExpansion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchical: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paging: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter: Option<Vec<TerminologyCapabilitiesExpansionParameter>>,
    #[serde(rename = "textFilter")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_filter: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesSoftware {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesCodeSystemVersion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(rename = "isDefault")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct TerminologyCapabilitiesCodeSystemVersionFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<Vec<Code>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "lockedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_date: Option<Boolean>,
    #[serde(rename = "codeSystem")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_system: Option<Vec<TerminologyCapabilitiesCodeSystem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expansion: Option<TerminologyCapabilitiesExpansion>,
    #[serde(rename = "codeSearch")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_search: Option<Code>,
    #[serde(rename = "validateCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_code: Option<TerminologyCapabilitiesValidateCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<TerminologyCapabilitiesTranslation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closure: Option<TerminologyCapabilitiesClosure>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesTranslation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "needsMap")]
    pub needs_map: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesImplementation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesValidateCode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub translations: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesCodeSystem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct TerminologyCapabilitiesClosure {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminologyCapabilitiesExpansionParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct TestReportSetupActionOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct TestReportSetup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestReportSetupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportTeardown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestReportTeardownAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportTeardownAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestReportSetupActionOperation,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportTestAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestReportSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportSetupActionAssert {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Markdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub uri: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestReportSetupAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestReportSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestReportSetupActionAssert>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptOrigin {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptTeardown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestScriptTeardownAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptVariable {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "defaultValue")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(rename = "headerField")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "sourceId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<Id>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptTestAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestScriptSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptMetadataCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct TestScriptTeardownAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestScriptSetupActionOperation,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<Vec<TestScriptMetadataLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<Vec<TestScriptMetadataCapability>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetup {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Vec<TestScriptSetupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptDestination {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetupActionOperation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accept: Option<Code>,
    #[serde(rename = "contentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "requestHeader")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_header: Option<Vec<TestScriptSetupActionOperationRequestHeader>>,
    #[serde(rename = "requestId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Id>,
    #[serde(rename = "responseId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<Id>,
    #[serde(rename = "sourceId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<Id>,
    #[serde(rename = "targetId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptMetadataLink {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetupAction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<TestScriptSetupActionOperation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetupActionOperationRequestHeader {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub field: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScript {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct TestScriptTest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct TestScriptFixture {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub autocreate: Boolean,
    pub autodelete: Boolean,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestScriptSetupActionAssert {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<Code>,
    #[serde(rename = "compareToSourceId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_to_source_id: Option<String>,
    #[serde(rename = "compareToSourceExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_to_source_expression: Option<String>,
    #[serde(rename = "compareToSourcePath")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_to_source_path: Option<String>,
    #[serde(rename = "contentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(rename = "headerField")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header_field: Option<String>,
    #[serde(rename = "minimumId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_id: Option<String>,
    #[serde(rename = "navigationLinks")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation_links: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "requestMethod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_method: Option<Code>,
    #[serde(rename = "requestURL")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_u_r_l: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Code>,
    #[serde(rename = "responseCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_code: Option<String>,
    #[serde(rename = "sourceId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<Id>,
    #[serde(rename = "validateProfileId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate_profile_id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(rename = "warningOnly")]
    pub warning_only: Boolean,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueSetExpansionParameterValue {
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Decimal(Decimal),
    Uri(Uri),
    Code(Code),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetExpansionParameter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_decimal: Option<Decimal>,
    #[serde(rename = "valueUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_uri: Option<Uri>,
    #[serde(rename = "valueCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Code>,
    #[serde(rename = "valueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date_time: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetExpansionContains {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(rename = "abstract")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ValueSetComposeInclude {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub concept: Option<Vec<ValueSetComposeIncludeConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Vec<ValueSetComposeIncludeFilter>>,
    #[serde(rename = "valueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Vec<Canonical>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "useContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ValueSetExpansion {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ValueSetComposeIncludeFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Code,
    pub op: Code,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetCompose {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "lockedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct ValueSetComposeIncludeConceptDesignation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(rename = "use")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Coding>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueSetComposeIncludeConcept {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResultAttestation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who: Option<Reference>,
    #[serde(rename = "onBehalfOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "communicationMethod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication_method: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(rename = "sourceIdentityCertificate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_identity_certificate: Option<String>,
    #[serde(rename = "proxyIdentityCertificate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_identity_certificate: Option<String>,
    #[serde(rename = "proxySignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_signature: Option<Signature>,
    #[serde(rename = "sourceSignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_signature: Option<Signature>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec<Reference>>,
    #[serde(rename = "targetLocation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_location: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub need: Option<CodeableConcept>,
    pub status: Code,
    #[serde(rename = "statusDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_date: Option<DateTime>,
    #[serde(rename = "validationType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_type: Option<CodeableConcept>,
    #[serde(rename = "validationProcess")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_process: Option<Vec<CodeableConcept>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<Timing>,
    #[serde(rename = "lastPerformed")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_performed: Option<DateTime>,
    #[serde(rename = "nextScheduled")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_scheduled: Option<Date>,
    #[serde(rename = "failureAction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_action: Option<CodeableConcept>,
    #[serde(rename = "primarySource")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub who: Option<Reference>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "communicationMethod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication_method: Option<Vec<CodeableConcept>>,
    #[serde(rename = "validationStatus")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_status: Option<CodeableConcept>,
    #[serde(rename = "validationDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_date: Option<DateTime>,
    #[serde(rename = "canPushUpdates")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_push_updates: Option<CodeableConcept>,
    #[serde(rename = "pushTypeAvailable")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_type_available: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResultValidator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub organization: Reference,
    #[serde(rename = "identityCertificate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_certificate: Option<String>,
    #[serde(rename = "attestationSignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attestation_signature: Option<Signature>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionPrescription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_rules: Option<Uri>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<Narrative>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contained: Option<Vec<Resource>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "lensSpecification")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lens_specification: Option<Vec<VisionPrescriptionLensSpecification>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionPrescriptionLensSpecificationPrism {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub amount: Decimal,
    pub base: Code,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionPrescriptionLensSpecification {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "backCurve")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "use")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Code>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "postalCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationAuthor {
    Reference(Reference),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Annotation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "authorReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_reference: Option<Reference>,
    #[serde(rename = "authorString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_string: Option<String>,
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
    #[serde(rename = "contentType")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "userSelected")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "use")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[serde(rename_all = "camelCase")]
pub enum DataRequirementDateFilterValue {
    DateTime(DateTime),
    Period(Period),
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
    #[serde(rename = "searchParam")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_param: Option<String>,
    #[serde(rename = "valueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valuePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_period: Option<Period>,
    #[serde(rename = "valueDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_duration: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementSubject {
    CodeableConcept(CodeableConcept),
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
    #[serde(rename = "subjectCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_reference: Option<Reference>,
    #[serde(rename = "mustSupport")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must_support: Option<Vec<String>>,
    #[serde(rename = "codeFilter")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_filter: Option<Vec<DataRequirementCodeFilter>>,
    #[serde(rename = "dateFilter")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
pub struct DataRequirementCodeFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(rename = "searchParam")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_param: Option<String>,
    #[serde(rename = "valueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Canonical>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<Vec<Coding>>,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageAsNeeded {
    Boolean(Boolean),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dosage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(rename = "additionalInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_instruction: Option<Vec<CodeableConcept>>,
    #[serde(rename = "patientInstruction")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient_instruction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
    #[serde(rename = "asNeededBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_needed_boolean: Option<Boolean>,
    #[serde(rename = "asNeededCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_needed_codeable_concept: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<CodeableConcept>,
    #[serde(rename = "doseAndRate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_and_rate: Option<Vec<DosageDoseAndRate>>,
    #[serde(rename = "maxDosePerPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dose_per_period: Option<Ratio>,
    #[serde(rename = "maxDosePerAdministration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dose_per_administration: Option<Quantity>,
    #[serde(rename = "maxDosePerLifetime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_dose_per_lifetime: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateDose {
    Range(Range),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateRate {
    Ratio(Ratio),
    Range(Range),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DosageDoseAndRate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "doseRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_range: Option<Range>,
    #[serde(rename = "doseQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dose_quantity: Option<Quantity>,
    #[serde(rename = "rateRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_ratio: Option<Ratio>,
    #[serde(rename = "rateRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_range: Option<Range>,
    #[serde(rename = "rateQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_quantity: Option<Quantity>,
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
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionDefaultValue {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionFixed {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionPattern {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMinValue {
    Date(Date),
    DateTime(DateTime),
    Instant(Instant),
    Time(Time),
    Decimal(Decimal),
    Integer(Integer),
    PositiveInt(PositiveInt),
    UnsignedInt(UnsignedInt),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMaxValue {
    Date(Date),
    DateTime(DateTime),
    Instant(Instant),
    Time(Time),
    Decimal(Decimal),
    Integer(Integer),
    PositiveInt(PositiveInt),
    UnsignedInt(UnsignedInt),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDefinition {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representation: Option<Vec<Code>>,
    #[serde(rename = "sliceName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slice_name: Option<String>,
    #[serde(rename = "sliceIsConstraining")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "contentReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_reference: Option<Uri>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<ElementDefinitionType>>,
    #[serde(rename = "defaultValueBase64Binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_base64_binary: Option<Base64Binary>,
    #[serde(rename = "defaultValueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_boolean: Option<Boolean>,
    #[serde(rename = "defaultValueCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_canonical: Option<Canonical>,
    #[serde(rename = "defaultValueCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_code: Option<Code>,
    #[serde(rename = "defaultValueDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_date: Option<Date>,
    #[serde(rename = "defaultValueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_date_time: Option<DateTime>,
    #[serde(rename = "defaultValueDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_decimal: Option<Decimal>,
    #[serde(rename = "defaultValueId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_id: Option<Id>,
    #[serde(rename = "defaultValueInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_instant: Option<Instant>,
    #[serde(rename = "defaultValueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_integer: Option<Integer>,
    #[serde(rename = "defaultValueMarkdown")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_markdown: Option<Markdown>,
    #[serde(rename = "defaultValueOid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_oid: Option<Oid>,
    #[serde(rename = "defaultValuePositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_positive_int: Option<PositiveInt>,
    #[serde(rename = "defaultValueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_string: Option<String>,
    #[serde(rename = "defaultValueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_time: Option<Time>,
    #[serde(rename = "defaultValueUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "defaultValueUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_uri: Option<Uri>,
    #[serde(rename = "defaultValueUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_url: Option<Url>,
    #[serde(rename = "defaultValueUuid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_uuid: Option<Uuid>,
    #[serde(rename = "defaultValueAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_address: Option<Address>,
    #[serde(rename = "defaultValueAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_age: Option<Age>,
    #[serde(rename = "defaultValueAnnotation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_annotation: Option<Annotation>,
    #[serde(rename = "defaultValueAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_attachment: Option<Attachment>,
    #[serde(rename = "defaultValueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "defaultValueCoding")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_coding: Option<Coding>,
    #[serde(rename = "defaultValueContactPoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_contact_point: Option<ContactPoint>,
    #[serde(rename = "defaultValueCount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_count: Option<Count>,
    #[serde(rename = "defaultValueDistance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_distance: Option<Distance>,
    #[serde(rename = "defaultValueDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_duration: Option<Duration>,
    #[serde(rename = "defaultValueHumanName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_human_name: Option<HumanName>,
    #[serde(rename = "defaultValueIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_identifier: Option<Identifier>,
    #[serde(rename = "defaultValueMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_money: Option<Money>,
    #[serde(rename = "defaultValuePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_period: Option<Period>,
    #[serde(rename = "defaultValueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_quantity: Option<Quantity>,
    #[serde(rename = "defaultValueRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_range: Option<Range>,
    #[serde(rename = "defaultValueRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_ratio: Option<Ratio>,
    #[serde(rename = "defaultValueReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_reference: Option<Reference>,
    #[serde(rename = "defaultValueSampledData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_sampled_data: Option<SampledData>,
    #[serde(rename = "defaultValueSignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_signature: Option<Signature>,
    #[serde(rename = "defaultValueTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_timing: Option<Timing>,
    #[serde(rename = "defaultValueContactDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_contact_detail: Option<ContactDetail>,
    #[serde(rename = "defaultValueContributor")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_contributor: Option<Contributor>,
    #[serde(rename = "defaultValueDataRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_data_requirement: Option<DataRequirement>,
    #[serde(rename = "defaultValueExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_expression: Option<Expression>,
    #[serde(rename = "defaultValueParameterDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "defaultValueRelatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "defaultValueTriggerDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "defaultValueUsageContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_usage_context: Option<UsageContext>,
    #[serde(rename = "defaultValueDosage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_dosage: Option<Dosage>,
    #[serde(rename = "defaultValueMeta")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value_meta: Option<Meta>,
    #[serde(rename = "meaningWhenMissing")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meaning_when_missing: Option<Markdown>,
    #[serde(rename = "orderMeaning")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_meaning: Option<String>,
    #[serde(rename = "fixedBase64Binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_base64_binary: Option<Base64Binary>,
    #[serde(rename = "fixedBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_boolean: Option<Boolean>,
    #[serde(rename = "fixedCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_canonical: Option<Canonical>,
    #[serde(rename = "fixedCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_code: Option<Code>,
    #[serde(rename = "fixedDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_date: Option<Date>,
    #[serde(rename = "fixedDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_date_time: Option<DateTime>,
    #[serde(rename = "fixedDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_decimal: Option<Decimal>,
    #[serde(rename = "fixedId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_id: Option<Id>,
    #[serde(rename = "fixedInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_instant: Option<Instant>,
    #[serde(rename = "fixedInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_integer: Option<Integer>,
    #[serde(rename = "fixedMarkdown")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_markdown: Option<Markdown>,
    #[serde(rename = "fixedOid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_oid: Option<Oid>,
    #[serde(rename = "fixedPositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_positive_int: Option<PositiveInt>,
    #[serde(rename = "fixedString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_string: Option<String>,
    #[serde(rename = "fixedTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_time: Option<Time>,
    #[serde(rename = "fixedUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "fixedUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_uri: Option<Uri>,
    #[serde(rename = "fixedUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_url: Option<Url>,
    #[serde(rename = "fixedUuid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_uuid: Option<Uuid>,
    #[serde(rename = "fixedAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_address: Option<Address>,
    #[serde(rename = "fixedAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_age: Option<Age>,
    #[serde(rename = "fixedAnnotation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_annotation: Option<Annotation>,
    #[serde(rename = "fixedAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_attachment: Option<Attachment>,
    #[serde(rename = "fixedCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "fixedCoding")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_coding: Option<Coding>,
    #[serde(rename = "fixedContactPoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_contact_point: Option<ContactPoint>,
    #[serde(rename = "fixedCount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_count: Option<Count>,
    #[serde(rename = "fixedDistance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_distance: Option<Distance>,
    #[serde(rename = "fixedDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_duration: Option<Duration>,
    #[serde(rename = "fixedHumanName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_human_name: Option<HumanName>,
    #[serde(rename = "fixedIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_identifier: Option<Identifier>,
    #[serde(rename = "fixedMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_money: Option<Money>,
    #[serde(rename = "fixedPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_period: Option<Period>,
    #[serde(rename = "fixedQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_quantity: Option<Quantity>,
    #[serde(rename = "fixedRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_range: Option<Range>,
    #[serde(rename = "fixedRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_ratio: Option<Ratio>,
    #[serde(rename = "fixedReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_reference: Option<Reference>,
    #[serde(rename = "fixedSampledData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_sampled_data: Option<SampledData>,
    #[serde(rename = "fixedSignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_signature: Option<Signature>,
    #[serde(rename = "fixedTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_timing: Option<Timing>,
    #[serde(rename = "fixedContactDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_contact_detail: Option<ContactDetail>,
    #[serde(rename = "fixedContributor")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_contributor: Option<Contributor>,
    #[serde(rename = "fixedDataRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_data_requirement: Option<DataRequirement>,
    #[serde(rename = "fixedExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_expression: Option<Expression>,
    #[serde(rename = "fixedParameterDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "fixedRelatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "fixedTriggerDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "fixedUsageContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_usage_context: Option<UsageContext>,
    #[serde(rename = "fixedDosage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_dosage: Option<Dosage>,
    #[serde(rename = "fixedMeta")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_meta: Option<Meta>,
    #[serde(rename = "patternBase64Binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_base64_binary: Option<Base64Binary>,
    #[serde(rename = "patternBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_boolean: Option<Boolean>,
    #[serde(rename = "patternCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_canonical: Option<Canonical>,
    #[serde(rename = "patternCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_code: Option<Code>,
    #[serde(rename = "patternDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_date: Option<Date>,
    #[serde(rename = "patternDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_date_time: Option<DateTime>,
    #[serde(rename = "patternDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_decimal: Option<Decimal>,
    #[serde(rename = "patternId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_id: Option<Id>,
    #[serde(rename = "patternInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_instant: Option<Instant>,
    #[serde(rename = "patternInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_integer: Option<Integer>,
    #[serde(rename = "patternMarkdown")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_markdown: Option<Markdown>,
    #[serde(rename = "patternOid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_oid: Option<Oid>,
    #[serde(rename = "patternPositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_positive_int: Option<PositiveInt>,
    #[serde(rename = "patternString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_string: Option<String>,
    #[serde(rename = "patternTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_time: Option<Time>,
    #[serde(rename = "patternUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "patternUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_uri: Option<Uri>,
    #[serde(rename = "patternUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_url: Option<Url>,
    #[serde(rename = "patternUuid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_uuid: Option<Uuid>,
    #[serde(rename = "patternAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_address: Option<Address>,
    #[serde(rename = "patternAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_age: Option<Age>,
    #[serde(rename = "patternAnnotation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_annotation: Option<Annotation>,
    #[serde(rename = "patternAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_attachment: Option<Attachment>,
    #[serde(rename = "patternCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "patternCoding")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_coding: Option<Coding>,
    #[serde(rename = "patternContactPoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_contact_point: Option<ContactPoint>,
    #[serde(rename = "patternCount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_count: Option<Count>,
    #[serde(rename = "patternDistance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_distance: Option<Distance>,
    #[serde(rename = "patternDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_duration: Option<Duration>,
    #[serde(rename = "patternHumanName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_human_name: Option<HumanName>,
    #[serde(rename = "patternIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_identifier: Option<Identifier>,
    #[serde(rename = "patternMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_money: Option<Money>,
    #[serde(rename = "patternPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_period: Option<Period>,
    #[serde(rename = "patternQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_quantity: Option<Quantity>,
    #[serde(rename = "patternRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_range: Option<Range>,
    #[serde(rename = "patternRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_ratio: Option<Ratio>,
    #[serde(rename = "patternReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_reference: Option<Reference>,
    #[serde(rename = "patternSampledData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_sampled_data: Option<SampledData>,
    #[serde(rename = "patternSignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_signature: Option<Signature>,
    #[serde(rename = "patternTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_timing: Option<Timing>,
    #[serde(rename = "patternContactDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_contact_detail: Option<ContactDetail>,
    #[serde(rename = "patternContributor")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_contributor: Option<Contributor>,
    #[serde(rename = "patternDataRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_data_requirement: Option<DataRequirement>,
    #[serde(rename = "patternExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_expression: Option<Expression>,
    #[serde(rename = "patternParameterDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "patternRelatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "patternTriggerDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "patternUsageContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_usage_context: Option<UsageContext>,
    #[serde(rename = "patternDosage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_dosage: Option<Dosage>,
    #[serde(rename = "patternMeta")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern_meta: Option<Meta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<Vec<ElementDefinitionExample>>,
    #[serde(rename = "minValueDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_date: Option<Date>,
    #[serde(rename = "minValueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_date_time: Option<DateTime>,
    #[serde(rename = "minValueInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_instant: Option<Instant>,
    #[serde(rename = "minValueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_time: Option<Time>,
    #[serde(rename = "minValueDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_decimal: Option<Decimal>,
    #[serde(rename = "minValueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_integer: Option<Integer>,
    #[serde(rename = "minValuePositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_positive_int: Option<PositiveInt>,
    #[serde(rename = "minValueUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "minValueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value_quantity: Option<Quantity>,
    #[serde(rename = "maxValueDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_date: Option<Date>,
    #[serde(rename = "maxValueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_date_time: Option<DateTime>,
    #[serde(rename = "maxValueInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_instant: Option<Instant>,
    #[serde(rename = "maxValueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_time: Option<Time>,
    #[serde(rename = "maxValueDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_decimal: Option<Decimal>,
    #[serde(rename = "maxValueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_integer: Option<Integer>,
    #[serde(rename = "maxValuePositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_positive_int: Option<PositiveInt>,
    #[serde(rename = "maxValueUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "maxValueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value_quantity: Option<Quantity>,
    #[serde(rename = "maxLength")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<Integer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Vec<Id>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraint: Option<Vec<ElementDefinitionConstraint>>,
    #[serde(rename = "mustSupport")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub must_support: Option<Boolean>,
    #[serde(rename = "isModifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_modifier: Option<Boolean>,
    #[serde(rename = "isModifierReason")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_modifier_reason: Option<String>,
    #[serde(rename = "isSummary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_summary: Option<Boolean>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<ElementDefinitionBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping: Option<Vec<ElementDefinitionMapping>>,
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
    #[serde(rename = "valueSet")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_set: Option<Canonical>,
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
    #[serde(rename = "targetProfile")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_profile: Option<Vec<Canonical>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregation: Option<Vec<Code>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versioning: Option<Code>,
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
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionExampleValue {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
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
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Base64Binary,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueCanonical")]
    pub value_canonical: Canonical,
    #[serde(rename = "valueCode")]
    pub value_code: Code,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
    #[serde(rename = "valueId")]
    pub value_id: Id,
    #[serde(rename = "valueInstant")]
    pub value_instant: Instant,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueMarkdown")]
    pub value_markdown: Markdown,
    #[serde(rename = "valueOid")]
    pub value_oid: Oid,
    #[serde(rename = "valuePositiveInt")]
    pub value_positive_int: PositiveInt,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueUnsignedInt")]
    pub value_unsigned_int: UnsignedInt,
    #[serde(rename = "valueUri")]
    pub value_uri: Uri,
    #[serde(rename = "valueUrl")]
    pub value_url: Url,
    #[serde(rename = "valueUuid")]
    pub value_uuid: Uuid,
    #[serde(rename = "valueAddress")]
    pub value_address: Address,
    #[serde(rename = "valueAge")]
    pub value_age: Age,
    #[serde(rename = "valueAnnotation")]
    pub value_annotation: Annotation,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueContactPoint")]
    pub value_contact_point: ContactPoint,
    #[serde(rename = "valueCount")]
    pub value_count: Count,
    #[serde(rename = "valueDistance")]
    pub value_distance: Distance,
    #[serde(rename = "valueDuration")]
    pub value_duration: Duration,
    #[serde(rename = "valueHumanName")]
    pub value_human_name: HumanName,
    #[serde(rename = "valueIdentifier")]
    pub value_identifier: Identifier,
    #[serde(rename = "valueMoney")]
    pub value_money: Money,
    #[serde(rename = "valuePeriod")]
    pub value_period: Period,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    #[serde(rename = "valueRatio")]
    pub value_ratio: Ratio,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    #[serde(rename = "valueSampledData")]
    pub value_sampled_data: SampledData,
    #[serde(rename = "valueSignature")]
    pub value_signature: Signature,
    #[serde(rename = "valueTiming")]
    pub value_timing: Timing,
    #[serde(rename = "valueContactDetail")]
    pub value_contact_detail: ContactDetail,
    #[serde(rename = "valueContributor")]
    pub value_contributor: Contributor,
    #[serde(rename = "valueDataRequirement")]
    pub value_data_requirement: DataRequirement,
    #[serde(rename = "valueExpression")]
    pub value_expression: Expression,
    #[serde(rename = "valueParameterDefinition")]
    pub value_parameter_definition: ParameterDefinition,
    #[serde(rename = "valueRelatedArtifact")]
    pub value_related_artifact: RelatedArtifact,
    #[serde(rename = "valueTriggerDefinition")]
    pub value_trigger_definition: TriggerDefinition,
    #[serde(rename = "valueUsageContext")]
    pub value_usage_context: UsageContext,
    #[serde(rename = "valueDosage")]
    pub value_dosage: Dosage,
    #[serde(rename = "valueMeta")]
    pub value_meta: Meta,
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionValue {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    Contributor(Contributor),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
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
    #[serde(rename = "valueBase64Binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_base64_binary: Option<Base64Binary>,
    #[serde(rename = "valueBoolean")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueCanonical")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_canonical: Option<Canonical>,
    #[serde(rename = "valueCode")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_code: Option<Code>,
    #[serde(rename = "valueDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valueDecimal")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_decimal: Option<Decimal>,
    #[serde(rename = "valueId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_id: Option<Id>,
    #[serde(rename = "valueInstant")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_instant: Option<Instant>,
    #[serde(rename = "valueInteger")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueMarkdown")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_markdown: Option<Markdown>,
    #[serde(rename = "valueOid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_oid: Option<Oid>,
    #[serde(rename = "valuePositiveInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_positive_int: Option<PositiveInt>,
    #[serde(rename = "valueString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(rename = "valueTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueUnsignedInt")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "valueUri")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_uri: Option<Uri>,
    #[serde(rename = "valueUrl")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_url: Option<Url>,
    #[serde(rename = "valueUuid")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_uuid: Option<Uuid>,
    #[serde(rename = "valueAddress")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_address: Option<Address>,
    #[serde(rename = "valueAge")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_age: Option<Age>,
    #[serde(rename = "valueAnnotation")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_annotation: Option<Annotation>,
    #[serde(rename = "valueAttachment")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueCoding")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_coding: Option<Coding>,
    #[serde(rename = "valueContactPoint")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_contact_point: Option<ContactPoint>,
    #[serde(rename = "valueCount")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_count: Option<Count>,
    #[serde(rename = "valueDistance")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_distance: Option<Distance>,
    #[serde(rename = "valueDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_duration: Option<Duration>,
    #[serde(rename = "valueHumanName")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_human_name: Option<HumanName>,
    #[serde(rename = "valueIdentifier")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_identifier: Option<Identifier>,
    #[serde(rename = "valueMoney")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_money: Option<Money>,
    #[serde(rename = "valuePeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_period: Option<Period>,
    #[serde(rename = "valueQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueRatio")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_ratio: Option<Ratio>,
    #[serde(rename = "valueReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_reference: Option<Reference>,
    #[serde(rename = "valueSampledData")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_sampled_data: Option<SampledData>,
    #[serde(rename = "valueSignature")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_signature: Option<Signature>,
    #[serde(rename = "valueTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_timing: Option<Timing>,
    #[serde(rename = "valueContactDetail")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_contact_detail: Option<ContactDetail>,
    #[serde(rename = "valueContributor")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_contributor: Option<Contributor>,
    #[serde(rename = "valueDataRequirement")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_data_requirement: Option<DataRequirement>,
    #[serde(rename = "valueExpression")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_expression: Option<Expression>,
    #[serde(rename = "valueParameterDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "valueRelatedArtifact")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "valueTriggerDefinition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "valueUsageContext")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_usage_context: Option<UsageContext>,
    #[serde(rename = "valueDosage")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_dosage: Option<Dosage>,
    #[serde(rename = "valueMeta")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_meta: Option<Meta>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanName {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "use")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#use: Option<Code>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<CodeableConcept>,
    pub status: CodeableConcept,
    #[serde(rename = "dateRange")]
    pub date_range: Period,
    #[serde(rename = "restoreDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_date: Option<DateTime>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "versionId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<Id>,
    #[serde(rename = "lastUpdated")]
    #[serde(skip_serializing_if = "Option::is_none")]
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


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PopulationAge {
    Range(Range),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Population {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "ageRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_range: Option<Range>,
    #[serde(rename = "ageCodeableConcept")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_codeable_concept: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub race: Option<CodeableConcept>,
    #[serde(rename = "physiologicalCondition")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physiological_condition: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProdCharacteristic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<Quantity>,
    #[serde(rename = "nominalVolume")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nominal_volume: Option<Quantity>,
    #[serde(rename = "externalDiameter")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub period: Quantity,
    #[serde(rename = "specialPrecautionsForStorage")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "lowerLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lower_limit: Option<Decimal>,
    #[serde(rename = "upperLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<Vec<Coding>>,
    pub when: Instant,
    pub who: Reference,
    #[serde(rename = "onBehalfOf")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "targetFormat")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_format: Option<Code>,
    #[serde(rename = "sigFormat")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sig_format: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Base64Binary>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceAmountReferenceRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "lowLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_limit: Option<Quantity>,
    #[serde(rename = "highLimit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_limit: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceAmountAmount {
    Quantity(Quantity),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceAmount {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "amountQuantity")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_quantity: Option<Quantity>,
    #[serde(rename = "amountRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_range: Option<Range>,
    #[serde(rename = "amountString")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_string: Option<String>,
    #[serde(rename = "amountType")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_type: Option<CodeableConcept>,
    #[serde(rename = "amountText")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_text: Option<String>,
    #[serde(rename = "referenceRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_range: Option<SubstanceAmountReferenceRange>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimingRepeatBounds {
    Duration(Duration),
    Range(Range),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimingRepeat {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<std::string::String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "boundsDuration")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds_duration: Option<Duration>,
    #[serde(rename = "boundsRange")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds_range: Option<Range>,
    #[serde(rename = "boundsPeriod")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds_period: Option<Period>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<PositiveInt>,
    #[serde(rename = "countMax")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count_max: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<Decimal>,
    #[serde(rename = "durationMax")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_max: Option<Decimal>,
    #[serde(rename = "durationUnit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_unit: Option<Code>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<PositiveInt>,
    #[serde(rename = "frequencyMax")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_max: Option<PositiveInt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Decimal>,
    #[serde(rename = "periodMax")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_max: Option<Decimal>,
    #[serde(rename = "periodUnit")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_unit: Option<Code>,
    #[serde(rename = "dayOfWeek")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day_of_week: Option<Vec<Code>>,
    #[serde(rename = "timeOfDay")]
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(rename = "modifierExtension")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Vec<DateTime>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<TimingRepeat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerDefinitionTiming {
    Timing(Timing),
    Reference(Reference),
    Date(Date),
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
    #[serde(rename = "timingTiming")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_timing: Option<Timing>,
    #[serde(rename = "timingReference")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_reference: Option<Reference>,
    #[serde(rename = "timingDate")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_date: Option<Date>,
    #[serde(rename = "timingDateTime")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing_date_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<DataRequirement>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Expression>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageContextValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Range(Range),
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
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
}


