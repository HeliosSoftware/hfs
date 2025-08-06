use helios_fhir_macro::{FhirSerde, FhirPath};
use serde::{Serialize, Deserialize};

use crate::{Element, DecimalElement};

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Account {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub name: Option<String>,
    pub subject: Option<Vec<Reference>>,
    #[fhir_serde(rename = "servicePeriod")]
    pub service_period: Option<Period>,
    pub coverage: Option<Vec<AccountCoverage>>,
    pub owner: Option<Reference>,
    pub description: Option<String>,
    pub guarantor: Option<Vec<AccountGuarantor>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AccountCoverage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    pub priority: Option<PositiveInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AccountGuarantor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub party: Reference,
    #[fhir_serde(rename = "onHold")]
    pub on_hold: Option<Boolean>,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ActivityDefinitionParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub role: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ActivityDefinitionDynamicValue {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub expression: Expression,
}

/// Choice of types for the subject\[x\] field in ActivityDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ActivityDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "subjectCanonical")]
    Canonical(Canonical),
}

/// Choice of types for the timing\[x\] field in ActivityDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ActivityDefinitionTiming {
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "timingDuration")]
    Duration(Duration),
}

/// Choice of types for the product\[x\] field in ActivityDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ActivityDefinitionProduct {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ActivityDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub subject: Option<ActivityDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub kind: Option<Code>,
    pub profile: Option<Canonical>,
    pub code: Option<CodeableConcept>,
    pub intent: Option<Code>,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub timing: Option<ActivityDefinitionTiming>,
    pub location: Option<Reference>,
    pub participant: Option<Vec<ActivityDefinitionParticipant>>,
    #[fhir_serde(flatten)]
    pub product: Option<ActivityDefinitionProduct>,
    pub quantity: Option<Quantity>,
    pub dosage: Option<Vec<Dosage>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "specimenRequirement")]
    pub specimen_requirement: Option<Vec<Reference>>,
    #[fhir_serde(rename = "observationRequirement")]
    pub observation_requirement: Option<Vec<Reference>>,
    #[fhir_serde(rename = "observationResultRequirement")]
    pub observation_result_requirement: Option<Vec<Reference>>,
    pub transform: Option<Canonical>,
    #[fhir_serde(rename = "dynamicValue")]
    pub dynamic_value: Option<Vec<ActivityDefinitionDynamicValue>>,
}


/// Choice of types for the value\[x\] field in AdministrableProductDefinitionProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum AdministrableProductDefinitionPropertyValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdministrableProductDefinitionProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<AdministrableProductDefinitionPropertyValue>,
    pub status: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdministrableProductDefinitionRouteOfAdministrationTargetSpecies {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(rename = "withdrawalPeriod")]
    pub withdrawal_period: Option<Vec<AdministrableProductDefinitionRouteOfAdministrationTargetSpeciesWithdrawalPeriod>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdministrableProductDefinitionRouteOfAdministrationTargetSpeciesWithdrawalPeriod {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub tissue: CodeableConcept,
    pub value: Quantity,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdministrableProductDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "formOf")]
    pub form_of: Option<Vec<Reference>>,
    #[fhir_serde(rename = "administrableDoseForm")]
    pub administrable_dose_form: Option<CodeableConcept>,
    #[fhir_serde(rename = "unitOfPresentation")]
    pub unit_of_presentation: Option<CodeableConcept>,
    #[fhir_serde(rename = "producedFrom")]
    pub produced_from: Option<Vec<Reference>>,
    pub ingredient: Option<Vec<CodeableConcept>>,
    pub device: Option<Reference>,
    pub property: Option<Vec<AdministrableProductDefinitionProperty>>,
    #[fhir_serde(rename = "routeOfAdministration")]
    pub route_of_administration: Option<Vec<AdministrableProductDefinitionRouteOfAdministration>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdministrableProductDefinitionRouteOfAdministration {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(rename = "firstDose")]
    pub first_dose: Option<Quantity>,
    #[fhir_serde(rename = "maxSingleDose")]
    pub max_single_dose: Option<Quantity>,
    #[fhir_serde(rename = "maxDosePerDay")]
    pub max_dose_per_day: Option<Quantity>,
    #[fhir_serde(rename = "maxDosePerTreatmentPeriod")]
    pub max_dose_per_treatment_period: Option<Ratio>,
    #[fhir_serde(rename = "maxTreatmentPeriod")]
    pub max_treatment_period: Option<Duration>,
    #[fhir_serde(rename = "targetSpecies")]
    pub target_species: Option<Vec<AdministrableProductDefinitionRouteOfAdministrationTargetSpecies>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdverseEventSuspectEntityCausality {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub assessment: Option<CodeableConcept>,
    #[fhir_serde(rename = "productRelatedness")]
    pub product_relatedness: Option<String>,
    pub author: Option<Reference>,
    pub method: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdverseEvent {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub actuality: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub event: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub date: Option<DateTime>,
    pub detected: Option<DateTime>,
    #[fhir_serde(rename = "recordedDate")]
    pub recorded_date: Option<DateTime>,
    #[fhir_serde(rename = "resultingCondition")]
    pub resulting_condition: Option<Vec<Reference>>,
    pub location: Option<Reference>,
    pub seriousness: Option<CodeableConcept>,
    pub severity: Option<CodeableConcept>,
    pub outcome: Option<CodeableConcept>,
    pub recorder: Option<Reference>,
    pub contributor: Option<Vec<Reference>>,
    #[fhir_serde(rename = "suspectEntity")]
    pub suspect_entity: Option<Vec<AdverseEventSuspectEntity>>,
    #[fhir_serde(rename = "subjectMedicalHistory")]
    pub subject_medical_history: Option<Vec<Reference>>,
    #[fhir_serde(rename = "referenceDocument")]
    pub reference_document: Option<Vec<Reference>>,
    pub study: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdverseEventSuspectEntity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub instance: Reference,
    pub causality: Option<Vec<AdverseEventSuspectEntityCausality>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AllergyIntoleranceReaction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: Option<CodeableConcept>,
    pub manifestation: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub onset: Option<DateTime>,
    pub severity: Option<Code>,
    #[fhir_serde(rename = "exposureRoute")]
    pub exposure_route: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the onset\[x\] field in AllergyIntolerance
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum AllergyIntoleranceOnset {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "onsetDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "onsetString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AllergyIntolerance {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "clinicalStatus")]
    pub clinical_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "verificationStatus")]
    pub verification_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub category: Option<Vec<Code>>,
    pub criticality: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub onset: Option<AllergyIntoleranceOnset>,
    #[fhir_serde(rename = "recordedDate")]
    pub recorded_date: Option<DateTime>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    #[fhir_serde(rename = "lastOccurrence")]
    pub last_occurrence: Option<DateTime>,
    pub note: Option<Vec<Annotation>>,
    pub reaction: Option<Vec<AllergyIntoleranceReaction>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Appointment {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "cancelationReason")]
    pub cancelation_reason: Option<CodeableConcept>,
    #[fhir_serde(rename = "serviceCategory")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "appointmentType")]
    pub appointment_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub priority: Option<UnsignedInt>,
    pub description: Option<String>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    pub start: Option<Instant>,
    pub end: Option<Instant>,
    #[fhir_serde(rename = "minutesDuration")]
    pub minutes_duration: Option<PositiveInt>,
    pub slot: Option<Vec<Reference>>,
    pub created: Option<DateTime>,
    pub comment: Option<String>,
    #[fhir_serde(rename = "patientInstruction")]
    pub patient_instruction: Option<String>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub participant: Option<Vec<AppointmentParticipant>>,
    #[fhir_serde(rename = "requestedPeriod")]
    pub requested_period: Option<Vec<Period>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AppointmentParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub actor: Option<Reference>,
    pub required: Option<Code>,
    pub status: Code,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AppointmentResponse {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub appointment: Reference,
    pub start: Option<Instant>,
    pub end: Option<Instant>,
    #[fhir_serde(rename = "participantType")]
    pub participant_type: Option<Vec<CodeableConcept>>,
    pub actor: Option<Reference>,
    #[fhir_serde(rename = "participantStatus")]
    pub participant_status: Code,
    pub comment: Option<String>,
}


/// Choice of types for the value\[x\] field in AuditEventEntityDetail
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum AuditEventEntityDetailValue {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEventEntityDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: String,
    #[fhir_serde(flatten)]
    pub value: Option<AuditEventEntityDetailValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEventEntity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub what: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Coding>,
    pub role: Option<Coding>,
    pub lifecycle: Option<Coding>,
    #[fhir_serde(rename = "securityLabel")]
    pub security_label: Option<Vec<Coding>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub query: Option<Base64Binary>,
    pub detail: Option<Vec<AuditEventEntityDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEvent {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Coding,
    pub subtype: Option<Vec<Coding>>,
    pub action: Option<Code>,
    pub period: Option<Period>,
    pub recorded: Instant,
    pub outcome: Option<Code>,
    #[fhir_serde(rename = "outcomeDesc")]
    pub outcome_desc: Option<String>,
    #[fhir_serde(rename = "purposeOfEvent")]
    pub purpose_of_event: Option<Vec<CodeableConcept>>,
    pub agent: Option<Vec<AuditEventAgent>>,
    pub source: AuditEventSource,
    pub entity: Option<Vec<AuditEventEntity>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEventAgentNetwork {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub address: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEventSource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub site: Option<String>,
    pub observer: Reference,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<Coding>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEventAgent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Option<Reference>,
    #[fhir_serde(rename = "altId")]
    pub alt_id: Option<String>,
    pub name: Option<String>,
    pub requestor: Boolean,
    pub location: Option<Reference>,
    pub policy: Option<Vec<Uri>>,
    pub media: Option<Coding>,
    pub network: Option<AuditEventAgentNetwork>,
    #[fhir_serde(rename = "purposeOfUse")]
    pub purpose_of_use: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Basic {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub created: Option<Date>,
    pub author: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Binary {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    #[fhir_serde(rename = "contentType")]
    pub content_type: Code,
    #[fhir_serde(rename = "securityContext")]
    pub security_context: Option<Reference>,
    pub data: Option<Base64Binary>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BiologicallyDerivedProductStorage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub temperature: Option<Decimal>,
    pub scale: Option<Code>,
    pub duration: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BiologicallyDerivedProduct {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "productCategory")]
    pub product_category: Option<Code>,
    #[fhir_serde(rename = "productCode")]
    pub product_code: Option<CodeableConcept>,
    pub status: Option<Code>,
    pub request: Option<Vec<Reference>>,
    pub quantity: Option<Integer>,
    pub parent: Option<Vec<Reference>>,
    pub collection: Option<BiologicallyDerivedProductCollection>,
    pub processing: Option<Vec<BiologicallyDerivedProductProcessing>>,
    pub manipulation: Option<BiologicallyDerivedProductManipulation>,
    pub storage: Option<Vec<BiologicallyDerivedProductStorage>>,
}

/// Choice of types for the collected\[x\] field in BiologicallyDerivedProductCollection
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum BiologicallyDerivedProductCollectionCollected {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "collectedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BiologicallyDerivedProductCollection {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub collector: Option<Reference>,
    pub source: Option<Reference>,
    #[fhir_serde(flatten)]
    pub collected: Option<BiologicallyDerivedProductCollectionCollected>,
}

/// Choice of types for the time\[x\] field in BiologicallyDerivedProductManipulation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum BiologicallyDerivedProductManipulationTime {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BiologicallyDerivedProductManipulation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    #[fhir_serde(flatten)]
    pub time: Option<BiologicallyDerivedProductManipulationTime>,
}

/// Choice of types for the time\[x\] field in BiologicallyDerivedProductProcessing
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum BiologicallyDerivedProductProcessingTime {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BiologicallyDerivedProductProcessing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub procedure: Option<CodeableConcept>,
    pub additive: Option<Reference>,
    #[fhir_serde(flatten)]
    pub time: Option<BiologicallyDerivedProductProcessingTime>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BodyStructure {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub morphology: Option<CodeableConcept>,
    pub location: Option<CodeableConcept>,
    #[fhir_serde(rename = "locationQualifier")]
    pub location_qualifier: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub image: Option<Vec<Attachment>>,
    pub patient: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Bundle {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub identifier: Option<Identifier>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub timestamp: Option<Instant>,
    pub total: Option<UnsignedInt>,
    pub link: Option<Vec<BundleLink>>,
    pub entry: Option<Vec<BundleEntry>>,
    pub signature: Option<Signature>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BundleEntryRequest {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub method: Code,
    pub url: Uri,
    #[fhir_serde(rename = "ifNoneMatch")]
    pub if_none_match: Option<String>,
    #[fhir_serde(rename = "ifModifiedSince")]
    pub if_modified_since: Option<Instant>,
    #[fhir_serde(rename = "ifMatch")]
    pub if_match: Option<String>,
    #[fhir_serde(rename = "ifNoneExist")]
    pub if_none_exist: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BundleEntryResponse {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: String,
    pub location: Option<Uri>,
    pub etag: Option<String>,
    #[fhir_serde(rename = "lastModified")]
    pub last_modified: Option<Instant>,
    pub outcome: Option<Resource>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BundleEntry {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub link: Option<Vec<BundleLink>>,
    #[fhir_serde(rename = "fullUrl")]
    pub full_url: Option<Uri>,
    pub resource: Option<Resource>,
    pub search: Option<BundleEntrySearch>,
    pub request: Option<BundleEntryRequest>,
    pub response: Option<BundleEntryResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BundleLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: String,
    pub url: Uri,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BundleEntrySearch {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Option<Code>,
    pub score: Option<Decimal>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatement {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub kind: Code,
    pub instantiates: Option<Vec<Canonical>>,
    pub imports: Option<Vec<Canonical>>,
    pub software: Option<CapabilityStatementSoftware>,
    pub implementation: Option<CapabilityStatementImplementation>,
    #[fhir_serde(rename = "fhirVersion")]
    pub fhir_version: Code,
    pub format: Option<Vec<Code>>,
    #[fhir_serde(rename = "patchFormat")]
    pub patch_format: Option<Vec<Code>>,
    #[fhir_serde(rename = "implementationGuide")]
    pub implementation_guide: Option<Vec<Canonical>>,
    pub rest: Option<Vec<CapabilityStatementRest>>,
    pub messaging: Option<Vec<CapabilityStatementMessaging>>,
    pub document: Option<Vec<CapabilityStatementDocument>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementSoftware {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub version: Option<String>,
    #[fhir_serde(rename = "releaseDate")]
    pub release_date: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementImplementation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub url: Option<Url>,
    pub custodian: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementMessaging {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub endpoint: Option<Vec<CapabilityStatementMessagingEndpoint>>,
    #[fhir_serde(rename = "reliableCache")]
    pub reliable_cache: Option<UnsignedInt>,
    pub documentation: Option<Markdown>,
    #[fhir_serde(rename = "supportedMessage")]
    pub supported_message: Option<Vec<CapabilityStatementMessagingSupportedMessage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementMessagingEndpoint {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub protocol: Coding,
    pub address: Url,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementRestResourceInteraction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementMessagingSupportedMessage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub definition: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementDocument {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub documentation: Option<Markdown>,
    pub profile: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementRestResourceSearchParam {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub definition: Option<Canonical>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementRestInteraction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementRest {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub documentation: Option<Markdown>,
    pub security: Option<CapabilityStatementRestSecurity>,
    pub resource: Option<Vec<CapabilityStatementRestResource>>,
    pub interaction: Option<Vec<CapabilityStatementRestInteraction>>,
    #[fhir_serde(rename = "searchParam")]
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
    pub compartment: Option<Vec<Canonical>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementRestResource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub profile: Option<Canonical>,
    #[fhir_serde(rename = "supportedProfile")]
    pub supported_profile: Option<Vec<Canonical>>,
    pub documentation: Option<Markdown>,
    pub interaction: Option<Vec<CapabilityStatementRestResourceInteraction>>,
    pub versioning: Option<Code>,
    #[fhir_serde(rename = "readHistory")]
    pub read_history: Option<Boolean>,
    #[fhir_serde(rename = "updateCreate")]
    pub update_create: Option<Boolean>,
    #[fhir_serde(rename = "conditionalCreate")]
    pub conditional_create: Option<Boolean>,
    #[fhir_serde(rename = "conditionalRead")]
    pub conditional_read: Option<Code>,
    #[fhir_serde(rename = "conditionalUpdate")]
    pub conditional_update: Option<Boolean>,
    #[fhir_serde(rename = "conditionalDelete")]
    pub conditional_delete: Option<Code>,
    #[fhir_serde(rename = "referencePolicy")]
    pub reference_policy: Option<Vec<Code>>,
    #[fhir_serde(rename = "searchInclude")]
    pub search_include: Option<Vec<String>>,
    #[fhir_serde(rename = "searchRevInclude")]
    pub search_rev_include: Option<Vec<String>>,
    #[fhir_serde(rename = "searchParam")]
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementRestResourceOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub definition: Canonical,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CapabilityStatementRestSecurity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub cors: Option<Boolean>,
    pub service: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
}


/// Choice of types for the scheduled\[x\] field in CarePlanActivityDetail
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CarePlanActivityDetailScheduled {
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "scheduledTiming")]
    Timing(Timing),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "scheduledPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "scheduledString")]
    String(String),
}

/// Choice of types for the product\[x\] field in CarePlanActivityDetail
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CarePlanActivityDetailProduct {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "productReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CarePlanActivityDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Option<Code>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub goal: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub scheduled: Option<CarePlanActivityDetailScheduled>,
    pub location: Option<Reference>,
    pub performer: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub product: Option<CarePlanActivityDetailProduct>,
    #[fhir_serde(rename = "dailyAmount")]
    pub daily_amount: Option<Quantity>,
    pub quantity: Option<Quantity>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CarePlanActivity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "outcomeCodeableConcept")]
    pub outcome_codeable_concept: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "outcomeReference")]
    pub outcome_reference: Option<Vec<Reference>>,
    pub progress: Option<Vec<Annotation>>,
    pub reference: Option<Reference>,
    pub detail: Option<CarePlanActivityDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CarePlan {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub period: Option<Period>,
    pub created: Option<DateTime>,
    pub author: Option<Reference>,
    pub contributor: Option<Vec<Reference>>,
    #[fhir_serde(rename = "careTeam")]
    pub care_team: Option<Vec<Reference>>,
    pub addresses: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub goal: Option<Vec<Reference>>,
    pub activity: Option<Vec<CarePlanActivity>>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CareTeamParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<Vec<CodeableConcept>>,
    pub member: Option<Reference>,
    #[fhir_serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CareTeam {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub period: Option<Period>,
    pub participant: Option<Vec<CareTeamParticipant>>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CatalogEntryRelatedEntry {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationtype: Code,
    pub item: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CatalogEntry {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub orderable: Boolean,
    #[fhir_serde(rename = "referencedItem")]
    pub referenced_item: Reference,
    #[fhir_serde(rename = "additionalIdentifier")]
    pub additional_identifier: Option<Vec<Identifier>>,
    pub classification: Option<Vec<CodeableConcept>>,
    pub status: Option<Code>,
    #[fhir_serde(rename = "validityPeriod")]
    pub validity_period: Option<Period>,
    #[fhir_serde(rename = "validTo")]
    pub valid_to: Option<DateTime>,
    #[fhir_serde(rename = "lastUpdated")]
    pub last_updated: Option<DateTime>,
    #[fhir_serde(rename = "additionalCharacteristic")]
    pub additional_characteristic: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "additionalClassification")]
    pub additional_classification: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "relatedEntry")]
    pub related_entry: Option<Vec<CatalogEntryRelatedEntry>>,
}


/// Choice of types for the occurrence\[x\] field in ChargeItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ChargeItemOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

/// Choice of types for the product\[x\] field in ChargeItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ChargeItemProduct {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ChargeItem {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "definitionUri")]
    pub definition_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "definitionCanonical")]
    pub definition_canonical: Option<Vec<Canonical>>,
    pub status: Code,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    pub context: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<ChargeItemOccurrence>,
    pub performer: Option<Vec<ChargeItemPerformer>>,
    #[fhir_serde(rename = "performingOrganization")]
    pub performing_organization: Option<Reference>,
    #[fhir_serde(rename = "requestingOrganization")]
    pub requesting_organization: Option<Reference>,
    #[fhir_serde(rename = "costCenter")]
    pub cost_center: Option<Reference>,
    pub quantity: Option<Quantity>,
    pub bodysite: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "factorOverride")]
    pub factor_override: Option<Decimal>,
    #[fhir_serde(rename = "priceOverride")]
    pub price_override: Option<Money>,
    #[fhir_serde(rename = "overrideReason")]
    pub override_reason: Option<String>,
    pub enterer: Option<Reference>,
    #[fhir_serde(rename = "enteredDate")]
    pub entered_date: Option<DateTime>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub service: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub product: Option<ChargeItemProduct>,
    pub account: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ChargeItemPerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ChargeItemDefinitionApplicability {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub expression: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ChargeItemDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "derivedFromUri")]
    pub derived_from_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Canonical>>,
    pub replaces: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub code: Option<CodeableConcept>,
    pub instance: Option<Vec<Reference>>,
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[fhir_serde(rename = "propertyGroup")]
    pub property_group: Option<Vec<ChargeItemDefinitionPropertyGroup>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ChargeItemDefinitionPropertyGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[fhir_serde(rename = "priceComponent")]
    pub price_component: Option<Vec<ChargeItemDefinitionPropertyGroupPriceComponent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ChargeItemDefinitionPropertyGroupPriceComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub code: Option<CodeableConcept>,
    pub factor: Option<Decimal>,
    pub amount: Option<Money>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactClassificationWhoClassified {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub person: Option<Reference>,
    pub organization: Option<Reference>,
    pub publisher: Option<Reference>,
    #[fhir_serde(rename = "classifierCopyright")]
    pub classifier_copyright: Option<String>,
    #[fhir_serde(rename = "freeToShare")]
    pub free_to_share: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactContributorshipEntry {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<HumanName>,
    pub initials: Option<String>,
    #[fhir_serde(rename = "collectiveName")]
    pub collective_name: Option<String>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "affiliationInfo")]
    pub affiliation_info: Option<Vec<CitationCitedArtifactContributorshipEntryAffiliationInfo>>,
    pub address: Option<Vec<Address>>,
    pub telecom: Option<Vec<ContactPoint>>,
    #[fhir_serde(rename = "contributionType")]
    pub contribution_type: Option<Vec<CodeableConcept>>,
    pub role: Option<CodeableConcept>,
    #[fhir_serde(rename = "contributionInstance")]
    pub contribution_instance: Option<Vec<CitationCitedArtifactContributorshipEntryContributionInstance>>,
    #[fhir_serde(rename = "correspondingContact")]
    pub corresponding_contact: Option<Boolean>,
    #[fhir_serde(rename = "listOrder")]
    pub list_order: Option<PositiveInt>,
}

/// Choice of types for the target\[x\] field in CitationRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CitationRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "targetIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "relationshipType")]
    pub relationship_type: CodeableConcept,
    #[fhir_serde(rename = "targetClassifier")]
    pub target_classifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub target: Option<CitationRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactPublicationFormPublishedIn {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub identifier: Option<Vec<Identifier>>,
    pub title: Option<String>,
    pub publisher: Option<Reference>,
    #[fhir_serde(rename = "publisherLocation")]
    pub publisher_location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactPublicationFormPeriodicRelease {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "citedMedium")]
    pub cited_medium: Option<CodeableConcept>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    #[fhir_serde(rename = "dateOfPublication")]
    pub date_of_publication: Option<CitationCitedArtifactPublicationFormPeriodicReleaseDateOfPublication>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifact {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "relatedIdentifier")]
    pub related_identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "dateAccessed")]
    pub date_accessed: Option<DateTime>,
    pub version: Option<CitationCitedArtifactVersion>,
    #[fhir_serde(rename = "currentState")]
    pub current_state: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<Vec<CitationCitedArtifactStatusDate>>,
    pub title: Option<Vec<CitationCitedArtifactTitle>>,
    #[fhir_serde(rename = "abstract")]
    pub r#abstract: Option<Vec<CitationCitedArtifactAbstract>>,
    pub part: Option<CitationCitedArtifactPart>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<CitationCitedArtifactRelatesTo>>,
    #[fhir_serde(rename = "publicationForm")]
    pub publication_form: Option<Vec<CitationCitedArtifactPublicationForm>>,
    #[fhir_serde(rename = "webLocation")]
    pub web_location: Option<Vec<CitationCitedArtifactWebLocation>>,
    pub classification: Option<Vec<CitationCitedArtifactClassification>>,
    pub contributorship: Option<CitationCitedArtifactContributorship>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationSummary {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub style: Option<CodeableConcept>,
    pub text: Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactContributorshipEntryAffiliationInfo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub affiliation: Option<String>,
    pub role: Option<String>,
    pub identifier: Option<Vec<Identifier>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactContributorshipSummary {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub style: Option<CodeableConcept>,
    pub source: Option<CodeableConcept>,
    pub value: Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactPublicationForm {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "publishedIn")]
    pub published_in: Option<CitationCitedArtifactPublicationFormPublishedIn>,
    #[fhir_serde(rename = "periodicRelease")]
    pub periodic_release: Option<CitationCitedArtifactPublicationFormPeriodicRelease>,
    #[fhir_serde(rename = "articleDate")]
    pub article_date: Option<DateTime>,
    #[fhir_serde(rename = "lastRevisionDate")]
    pub last_revision_date: Option<DateTime>,
    pub language: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "accessionNumber")]
    pub accession_number: Option<String>,
    #[fhir_serde(rename = "pageString")]
    pub page_string: Option<String>,
    #[fhir_serde(rename = "firstPage")]
    pub first_page: Option<String>,
    #[fhir_serde(rename = "lastPage")]
    pub last_page: Option<String>,
    #[fhir_serde(rename = "pageCount")]
    pub page_count: Option<String>,
    pub copyright: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactTitle {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub language: Option<CodeableConcept>,
    pub text: Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactClassification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub classifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "whoClassified")]
    pub who_classified: Option<CitationCitedArtifactClassificationWhoClassified>,
}

/// Choice of types for the target\[x\] field in CitationCitedArtifactRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CitationCitedArtifactRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "targetIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "relationshipType")]
    pub relationship_type: CodeableConcept,
    #[fhir_serde(rename = "targetClassifier")]
    pub target_classifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub target: Option<CitationCitedArtifactRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactPart {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub value: Option<String>,
    #[fhir_serde(rename = "baseCitation")]
    pub base_citation: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactPublicationFormPeriodicReleaseDateOfPublication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<Date>,
    pub year: Option<String>,
    pub month: Option<String>,
    pub day: Option<String>,
    pub season: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationStatusDate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub activity: CodeableConcept,
    pub actual: Option<Boolean>,
    pub period: Period,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactWebLocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub url: Option<Uri>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Citation {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub summary: Option<Vec<CitationSummary>>,
    pub classification: Option<Vec<CitationClassification>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "currentState")]
    pub current_state: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<Vec<CitationStatusDate>>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<CitationRelatesTo>>,
    #[fhir_serde(rename = "citedArtifact")]
    pub cited_artifact: Option<CitationCitedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactStatusDate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub activity: CodeableConcept,
    pub actual: Option<Boolean>,
    pub period: Period,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactContributorship {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub complete: Option<Boolean>,
    pub entry: Option<Vec<CitationCitedArtifactContributorshipEntry>>,
    pub summary: Option<Vec<CitationCitedArtifactContributorshipSummary>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactContributorshipEntryContributionInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub time: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactVersion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: String,
    #[fhir_serde(rename = "baseCitation")]
    pub base_citation: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactAbstract {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub language: Option<CodeableConcept>,
    pub text: Markdown,
    pub copyright: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationClassification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub classifier: Option<Vec<CodeableConcept>>,
}


/// Choice of types for the diagnosis\[x\] field in ClaimDiagnosis
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimDiagnosis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(flatten)]
    pub diagnosis: Option<ClaimDiagnosisDiagnosis>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "onAdmission")]
    pub on_admission: Option<CodeableConcept>,
    #[fhir_serde(rename = "packageCode")]
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimCareTeam {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub provider: Reference,
    pub responsible: Option<Boolean>,
    pub role: Option<CodeableConcept>,
    pub qualification: Option<CodeableConcept>,
}

/// Choice of types for the procedure\[x\] field in ClaimProcedure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimProcedureProcedure {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "procedureReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimProcedure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub date: Option<DateTime>,
    #[fhir_serde(flatten)]
    pub procedure: Option<ClaimProcedureProcedure>,
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimRelated {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub claim: Option<Reference>,
    pub relationship: Option<CodeableConcept>,
    pub reference: Option<Identifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Claim {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    #[fhir_serde(rename = "billablePeriod")]
    pub billable_period: Option<Period>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub insurer: Option<Reference>,
    pub provider: Reference,
    pub priority: CodeableConcept,
    #[fhir_serde(rename = "fundsReserve")]
    pub funds_reserve: Option<CodeableConcept>,
    pub related: Option<Vec<ClaimRelated>>,
    pub prescription: Option<Reference>,
    #[fhir_serde(rename = "originalPrescription")]
    pub original_prescription: Option<Reference>,
    pub payee: Option<ClaimPayee>,
    pub referral: Option<Reference>,
    pub facility: Option<Reference>,
    #[fhir_serde(rename = "careTeam")]
    pub care_team: Option<Vec<ClaimCareTeam>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<ClaimSupportingInfo>>,
    pub diagnosis: Option<Vec<ClaimDiagnosis>>,
    pub procedure: Option<Vec<ClaimProcedure>>,
    pub insurance: Option<Vec<ClaimInsurance>>,
    pub accident: Option<ClaimAccident>,
    pub item: Option<Vec<ClaimItem>>,
    pub total: Option<Money>,
}

/// Choice of types for the location\[x\] field in ClaimAccident
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimAccidentLocation {
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimAccident {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Date,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub location: Option<ClaimAccidentLocation>,
}

/// Choice of types for the timing\[x\] field in ClaimSupportingInfo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimSupportingInfoTiming {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timingPeriod")]
    Period(Period),
}

/// Choice of types for the value\[x\] field in ClaimSupportingInfo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimSupportingInfoValue {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimSupportingInfo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub timing: Option<ClaimSupportingInfoTiming>,
    #[fhir_serde(flatten)]
    pub value: Option<ClaimSupportingInfoValue>,
    pub reason: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimPayee {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub party: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimInsurance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub identifier: Option<Identifier>,
    pub coverage: Reference,
    #[fhir_serde(rename = "businessArrangement")]
    pub business_arrangement: Option<String>,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[fhir_serde(rename = "claimResponse")]
    pub claim_response: Option<Reference>,
}

/// Choice of types for the serviced\[x\] field in ClaimItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimItemServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location\[x\] field in ClaimItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimItemLocation {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "careTeamSequence")]
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "diagnosisSequence")]
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "procedureSequence")]
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "informationSequence")]
    pub information_sequence: Option<Vec<PositiveInt>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub serviced: Option<ClaimItemServiced>,
    #[fhir_serde(flatten)]
    pub location: Option<ClaimItemLocation>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[fhir_serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    pub encounter: Option<Vec<Reference>>,
    pub detail: Option<Vec<ClaimItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ClaimItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseItemAdjudication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub reason: Option<CodeableConcept>,
    pub amount: Option<Money>,
    pub value: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "detailSequence")]
    pub detail_sequence: PositiveInt,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ClaimResponseItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseAddItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponse {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    pub created: DateTime,
    pub insurer: Reference,
    pub requestor: Option<Reference>,
    pub request: Option<Reference>,
    pub outcome: Code,
    pub disposition: Option<String>,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<String>,
    #[fhir_serde(rename = "preAuthPeriod")]
    pub pre_auth_period: Option<Period>,
    #[fhir_serde(rename = "payeeType")]
    pub payee_type: Option<CodeableConcept>,
    pub item: Option<Vec<ClaimResponseItem>>,
    #[fhir_serde(rename = "addItem")]
    pub add_item: Option<Vec<ClaimResponseAddItem>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub total: Option<Vec<ClaimResponseTotal>>,
    pub payment: Option<ClaimResponsePayment>,
    #[fhir_serde(rename = "fundsReserve")]
    pub funds_reserve: Option<CodeableConcept>,
    #[fhir_serde(rename = "formCode")]
    pub form_code: Option<CodeableConcept>,
    pub form: Option<Attachment>,
    #[fhir_serde(rename = "processNote")]
    pub process_note: Option<Vec<ClaimResponseProcessNote>>,
    #[fhir_serde(rename = "communicationRequest")]
    pub communication_request: Option<Vec<Reference>>,
    pub insurance: Option<Vec<ClaimResponseInsurance>>,
    pub error: Option<Vec<ClaimResponseError>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponsePayment {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub adjustment: Option<Money>,
    #[fhir_serde(rename = "adjustmentReason")]
    pub adjustment_reason: Option<CodeableConcept>,
    pub date: Option<Date>,
    pub amount: Money,
    pub identifier: Option<Identifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseProcessNote {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<PositiveInt>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: String,
    pub language: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseInsurance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub coverage: Reference,
    #[fhir_serde(rename = "businessArrangement")]
    pub business_arrangement: Option<String>,
    #[fhir_serde(rename = "claimResponse")]
    pub claim_response: Option<Reference>,
}

/// Choice of types for the serviced\[x\] field in ClaimResponseAddItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimResponseAddItemServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location\[x\] field in ClaimResponseAddItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClaimResponseAddItemLocation {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseAddItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "itemSequence")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "detailSequence")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "subdetailSequence")]
    pub subdetail_sequence: Option<Vec<PositiveInt>>,
    pub provider: Option<Vec<Reference>>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub serviced: Option<ClaimResponseAddItemServiced>,
    #[fhir_serde(flatten)]
    pub location: Option<ClaimResponseAddItemLocation>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[fhir_serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub detail: Option<Vec<ClaimResponseAddItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "itemSequence")]
    pub item_sequence: PositiveInt,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub detail: Option<Vec<ClaimResponseItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseAddItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ClaimResponseAddItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseTotal {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseError {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "itemSequence")]
    pub item_sequence: Option<PositiveInt>,
    #[fhir_serde(rename = "detailSequence")]
    pub detail_sequence: Option<PositiveInt>,
    #[fhir_serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: Option<PositiveInt>,
    pub code: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: PositiveInt,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalImpressionFinding {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: Option<CodeableConcept>,
    #[fhir_serde(rename = "itemReference")]
    pub item_reference: Option<Reference>,
    pub basis: Option<String>,
}

/// Choice of types for the effective\[x\] field in ClinicalImpression
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClinicalImpressionEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalImpression {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub effective: Option<ClinicalImpressionEffective>,
    pub date: Option<DateTime>,
    pub assessor: Option<Reference>,
    pub previous: Option<Reference>,
    pub problem: Option<Vec<Reference>>,
    pub investigation: Option<Vec<ClinicalImpressionInvestigation>>,
    pub protocol: Option<Vec<Uri>>,
    pub summary: Option<String>,
    pub finding: Option<Vec<ClinicalImpressionFinding>>,
    #[fhir_serde(rename = "prognosisCodeableConcept")]
    pub prognosis_codeable_concept: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "prognosisReference")]
    pub prognosis_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalImpressionInvestigation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub item: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinitionContraindicationOtherTherapy {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "relationshipType")]
    pub relationship_type: CodeableConcept,
    pub therapy: CodeableReference,
}

/// Choice of types for the item\[x\] field in ClinicalUseDefinitionInteractionInteractant
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClinicalUseDefinitionInteractionInteractantItem {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "itemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinitionInteractionInteractant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub item: Option<ClinicalUseDefinitionInteractionInteractantItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinitionUndesirableEffect {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "symptomConditionEffect")]
    pub symptom_condition_effect: Option<CodeableReference>,
    pub classification: Option<CodeableConcept>,
    #[fhir_serde(rename = "frequencyOfOccurrence")]
    pub frequency_of_occurrence: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinitionWarning {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinitionContraindication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "diseaseSymptomProcedure")]
    pub disease_symptom_procedure: Option<CodeableReference>,
    #[fhir_serde(rename = "diseaseStatus")]
    pub disease_status: Option<CodeableReference>,
    pub comorbidity: Option<Vec<CodeableReference>>,
    pub indication: Option<Vec<Reference>>,
    #[fhir_serde(rename = "otherTherapy")]
    pub other_therapy: Option<Vec<ClinicalUseDefinitionContraindicationOtherTherapy>>,
}

/// Choice of types for the duration\[x\] field in ClinicalUseDefinitionIndication
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ClinicalUseDefinitionIndicationDuration {
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "durationRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "durationString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinitionIndication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "diseaseSymptomProcedure")]
    pub disease_symptom_procedure: Option<CodeableReference>,
    #[fhir_serde(rename = "diseaseStatus")]
    pub disease_status: Option<CodeableReference>,
    pub comorbidity: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "intendedEffect")]
    pub intended_effect: Option<CodeableReference>,
    #[fhir_serde(flatten)]
    pub duration: Option<ClinicalUseDefinitionIndicationDuration>,
    #[fhir_serde(rename = "undesirableEffect")]
    pub undesirable_effect: Option<Vec<Reference>>,
    #[fhir_serde(rename = "otherTherapy")]
    pub other_therapy: Option<Vec<ClinicalUseDefinitionContraindicationOtherTherapy>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Vec<Reference>>,
    pub status: Option<CodeableConcept>,
    pub contraindication: Option<ClinicalUseDefinitionContraindication>,
    pub indication: Option<ClinicalUseDefinitionIndication>,
    pub interaction: Option<ClinicalUseDefinitionInteraction>,
    pub population: Option<Vec<Reference>>,
    #[fhir_serde(rename = "undesirableEffect")]
    pub undesirable_effect: Option<ClinicalUseDefinitionUndesirableEffect>,
    pub warning: Option<ClinicalUseDefinitionWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinitionInteraction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub interactant: Option<Vec<ClinicalUseDefinitionInteractionInteractant>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub effect: Option<CodeableReference>,
    pub incidence: Option<CodeableConcept>,
    pub management: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CodeSystemConcept {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub display: Option<String>,
    pub definition: Option<String>,
    pub designation: Option<Vec<CodeSystemConceptDesignation>>,
    pub property: Option<Vec<CodeSystemConceptProperty>>,
    pub concept: Option<Vec<CodeSystemConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CodeSystemProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub uri: Option<Uri>,
    pub description: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CodeSystemConceptDesignation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<Code>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Coding>,
    pub value: String,
}

/// Choice of types for the value\[x\] field in CodeSystemConceptProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CodeSystemConceptPropertyValue {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CodeSystemConceptProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[fhir_serde(flatten)]
    pub value: Option<CodeSystemConceptPropertyValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CodeSystemFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub description: Option<String>,
    pub operator: Option<Vec<Code>>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CodeSystem {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "caseSensitive")]
    pub case_sensitive: Option<Boolean>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    #[fhir_serde(rename = "hierarchyMeaning")]
    pub hierarchy_meaning: Option<Code>,
    pub compositional: Option<Boolean>,
    #[fhir_serde(rename = "versionNeeded")]
    pub version_needed: Option<Boolean>,
    pub content: Code,
    pub supplements: Option<Canonical>,
    pub count: Option<UnsignedInt>,
    pub filter: Option<Vec<CodeSystemFilter>>,
    pub property: Option<Vec<CodeSystemProperty>>,
    pub concept: Option<Vec<CodeSystemConcept>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Communication {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    #[fhir_serde(rename = "inResponseTo")]
    pub in_response_to: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    pub medium: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub topic: Option<CodeableConcept>,
    pub about: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    pub sent: Option<DateTime>,
    pub received: Option<DateTime>,
    pub recipient: Option<Vec<Reference>>,
    pub sender: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub payload: Option<Vec<CommunicationPayload>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the content\[x\] field in CommunicationPayload
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CommunicationPayloadContent {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "contentString")]
    String(String),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CommunicationPayload {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<CommunicationPayloadContent>,
}


/// Choice of types for the occurrence\[x\] field in CommunicationRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CommunicationRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CommunicationRequest {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    pub medium: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub about: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    pub payload: Option<Vec<CommunicationRequestPayload>>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<CommunicationRequestOccurrence>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub recipient: Option<Vec<Reference>>,
    pub sender: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the content\[x\] field in CommunicationRequestPayload
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CommunicationRequestPayloadContent {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "contentString")]
    String(String),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CommunicationRequestPayload {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<CommunicationRequestPayloadContent>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CompartmentDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub version: Option<String>,
    pub name: String,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub purpose: Option<Markdown>,
    pub code: Code,
    pub search: Boolean,
    pub resource: Option<Vec<CompartmentDefinitionResource>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CompartmentDefinitionResource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub param: Option<Vec<String>>,
    pub documentation: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Composition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub date: DateTime,
    pub author: Option<Vec<Reference>>,
    pub title: String,
    pub confidentiality: Option<Code>,
    pub attester: Option<Vec<CompositionAttester>>,
    pub custodian: Option<Reference>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<CompositionRelatesTo>>,
    pub event: Option<Vec<CompositionEvent>>,
    pub section: Option<Vec<CompositionSection>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CompositionAttester {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub time: Option<DateTime>,
    pub party: Option<Reference>,
}

/// Choice of types for the target\[x\] field in CompositionRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CompositionRelatesToTarget {
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "targetIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CompositionRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[fhir_serde(flatten)]
    pub target: Option<CompositionRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CompositionSection {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub code: Option<CodeableConcept>,
    pub author: Option<Vec<Reference>>,
    pub focus: Option<Reference>,
    pub text: Option<Narrative>,
    pub mode: Option<Code>,
    #[fhir_serde(rename = "orderedBy")]
    pub ordered_by: Option<CodeableConcept>,
    pub entry: Option<Vec<Reference>>,
    #[fhir_serde(rename = "emptyReason")]
    pub empty_reason: Option<CodeableConcept>,
    pub section: Option<Vec<CompositionSection>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CompositionEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub detail: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMapGroupUnmapped {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub url: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMapGroupElementTarget {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub equivalence: Code,
    pub comment: Option<String>,
    #[fhir_serde(rename = "dependsOn")]
    pub depends_on: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
    pub product: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMapGroupElement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub target: Option<Vec<ConceptMapGroupElementTarget>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMapGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: Option<Uri>,
    #[fhir_serde(rename = "sourceVersion")]
    pub source_version: Option<String>,
    pub target: Option<Uri>,
    #[fhir_serde(rename = "targetVersion")]
    pub target_version: Option<String>,
    pub element: Option<Vec<ConceptMapGroupElement>>,
    pub unmapped: Option<ConceptMapGroupUnmapped>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMapGroupElementTargetDependsOn {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Uri,
    pub system: Option<Canonical>,
    pub value: String,
    pub display: Option<String>,
}

/// Choice of types for the source\[x\] field in ConceptMap
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ConceptMapSource {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "sourceUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "sourceCanonical")]
    Canonical(Canonical),
}

/// Choice of types for the target\[x\] field in ConceptMap
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ConceptMapTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMap {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Identifier>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(flatten)]
    pub source: Option<ConceptMapSource>,
    #[fhir_serde(flatten)]
    pub target: Option<ConceptMapTarget>,
    pub group: Option<Vec<ConceptMapGroup>>,
}


/// Choice of types for the onset\[x\] field in Condition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ConditionOnset {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "onsetDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "onsetString")]
    String(String),
}

/// Choice of types for the abatement\[x\] field in Condition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ConditionAbatement {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "abatementDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "abatementAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "abatementPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "abatementRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "abatementString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Condition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "clinicalStatus")]
    pub clinical_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "verificationStatus")]
    pub verification_status: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub severity: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub onset: Option<ConditionOnset>,
    #[fhir_serde(flatten)]
    pub abatement: Option<ConditionAbatement>,
    #[fhir_serde(rename = "recordedDate")]
    pub recorded_date: Option<DateTime>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    pub stage: Option<Vec<ConditionStage>>,
    pub evidence: Option<Vec<ConditionEvidence>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConditionEvidence {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub detail: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConditionStage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub summary: Option<CodeableConcept>,
    pub assessment: Option<Vec<Reference>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConsentPolicy {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub authority: Option<Uri>,
    pub uri: Option<Uri>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConsentProvisionData {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub meaning: Code,
    pub reference: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConsentProvision {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub period: Option<Period>,
    pub actor: Option<Vec<ConsentProvisionActor>>,
    pub action: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "securityLabel")]
    pub security_label: Option<Vec<Coding>>,
    pub purpose: Option<Vec<Coding>>,
    pub class: Option<Vec<Coding>>,
    pub code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "dataPeriod")]
    pub data_period: Option<Period>,
    pub data: Option<Vec<ConsentProvisionData>>,
    pub provision: Option<Vec<ConsentProvision>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConsentProvisionActor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: CodeableConcept,
    pub reference: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConsentVerification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub verified: Boolean,
    #[fhir_serde(rename = "verifiedWith")]
    pub verified_with: Option<Reference>,
    #[fhir_serde(rename = "verificationDate")]
    pub verification_date: Option<DateTime>,
}

/// Choice of types for the source\[x\] field in Consent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ConsentSource {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "sourceAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "sourceReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Consent {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub scope: CodeableConcept,
    pub category: Option<Vec<CodeableConcept>>,
    pub patient: Option<Reference>,
    #[fhir_serde(rename = "dateTime")]
    pub date_time: Option<DateTime>,
    pub performer: Option<Vec<Reference>>,
    pub organization: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub source: Option<ConsentSource>,
    pub policy: Option<Vec<ConsentPolicy>>,
    #[fhir_serde(rename = "policyRule")]
    pub policy_rule: Option<CodeableConcept>,
    pub verification: Option<Vec<ConsentVerification>>,
    pub provision: Option<ConsentProvision>,
}


/// Choice of types for the topic\[x\] field in ContractTerm
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractTermTopic {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "topicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "topicReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTerm {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub issued: Option<DateTime>,
    pub applies: Option<Period>,
    #[fhir_serde(flatten)]
    pub topic: Option<ContractTermTopic>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    pub text: Option<String>,
    #[fhir_serde(rename = "securityLabel")]
    pub security_label: Option<Vec<ContractTermSecurityLabel>>,
    pub offer: ContractTermOffer,
    pub asset: Option<Vec<ContractTermAsset>>,
    pub action: Option<Vec<ContractTermAction>>,
    pub group: Option<Vec<ContractTerm>>,
}

/// Choice of types for the content\[x\] field in ContractRule
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractRuleContent {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractRule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<ContractRuleContent>,
}

/// Choice of types for the content\[x\] field in ContractFriendly
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractFriendlyContent {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractFriendly {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<ContractFriendlyContent>,
}

/// Choice of types for the value\[x\] field in ContractTermOfferAnswer
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractTermOfferAnswerValue {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermOfferAnswer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<ContractTermOfferAnswerValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermOfferParty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Vec<Reference>>,
    pub role: CodeableConcept,
}

/// Choice of types for the occurrence\[x\] field in ContractTermAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractTermActionOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub subject: Option<Vec<ContractTermActionSubject>>,
    pub intent: CodeableConcept,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<Vec<String>>,
    pub status: CodeableConcept,
    pub context: Option<Reference>,
    #[fhir_serde(rename = "contextLinkId")]
    pub context_link_id: Option<Vec<String>>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<ContractTermActionOccurrence>,
    pub requester: Option<Vec<Reference>>,
    #[fhir_serde(rename = "requesterLinkId")]
    pub requester_link_id: Option<Vec<String>>,
    #[fhir_serde(rename = "performerType")]
    pub performer_type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "performerRole")]
    pub performer_role: Option<CodeableConcept>,
    pub performer: Option<Reference>,
    #[fhir_serde(rename = "performerLinkId")]
    pub performer_link_id: Option<Vec<String>>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub reason: Option<Vec<String>>,
    #[fhir_serde(rename = "reasonLinkId")]
    pub reason_link_id: Option<Vec<String>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

/// Choice of types for the topic\[x\] field in Contract
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractTopic {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "topicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "topicReference")]
    Reference(Reference),
}

/// Choice of types for the legallyBinding\[x\] field in Contract
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractLegallyBinding {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "legallyBindingAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "legallyBindingReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Contract {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub status: Option<Code>,
    #[fhir_serde(rename = "legalState")]
    pub legal_state: Option<CodeableConcept>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Reference>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Uri>,
    #[fhir_serde(rename = "contentDerivative")]
    pub content_derivative: Option<CodeableConcept>,
    pub issued: Option<DateTime>,
    pub applies: Option<Period>,
    #[fhir_serde(rename = "expirationType")]
    pub expiration_type: Option<CodeableConcept>,
    pub subject: Option<Vec<Reference>>,
    pub authority: Option<Vec<Reference>>,
    pub domain: Option<Vec<Reference>>,
    pub site: Option<Vec<Reference>>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub alias: Option<Vec<String>>,
    pub author: Option<Reference>,
    pub scope: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub topic: Option<ContractTopic>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "subType")]
    pub sub_type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "contentDefinition")]
    pub content_definition: Option<ContractContentDefinition>,
    pub term: Option<Vec<ContractTerm>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    #[fhir_serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
    pub signer: Option<Vec<ContractSigner>>,
    pub friendly: Option<Vec<ContractFriendly>>,
    pub legal: Option<Vec<ContractLegal>>,
    pub rule: Option<Vec<ContractRule>>,
    #[fhir_serde(flatten)]
    pub legally_binding: Option<ContractLegallyBinding>,
}

/// Choice of types for the content\[x\] field in ContractLegal
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractLegalContent {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractLegal {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<ContractLegalContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractContentDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    pub publisher: Option<Reference>,
    #[fhir_serde(rename = "publicationDate")]
    pub publication_date: Option<DateTime>,
    #[fhir_serde(rename = "publicationStatus")]
    pub publication_status: Code,
    pub copyright: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermOffer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub party: Option<Vec<ContractTermOfferParty>>,
    pub topic: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub decision: Option<CodeableConcept>,
    #[fhir_serde(rename = "decisionMode")]
    pub decision_mode: Option<Vec<CodeableConcept>>,
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    pub text: Option<String>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<Vec<String>>,
    #[fhir_serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermAsset {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub scope: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "typeReference")]
    pub type_reference: Option<Vec<Reference>>,
    pub subtype: Option<Vec<CodeableConcept>>,
    pub relationship: Option<Coding>,
    pub context: Option<Vec<ContractTermAssetContext>>,
    pub condition: Option<String>,
    #[fhir_serde(rename = "periodType")]
    pub period_type: Option<Vec<CodeableConcept>>,
    pub period: Option<Vec<Period>>,
    #[fhir_serde(rename = "usePeriod")]
    pub use_period: Option<Vec<Period>>,
    pub text: Option<String>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<Vec<String>>,
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    #[fhir_serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
    #[fhir_serde(rename = "valuedItem")]
    pub valued_item: Option<Vec<ContractTermAssetValuedItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermActionSubject {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Vec<Reference>>,
    pub role: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractSigner {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Coding,
    pub party: Reference,
    pub signature: Option<Vec<Signature>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermAssetContext {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Reference>,
    pub code: Option<Vec<CodeableConcept>>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermSecurityLabel {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<Vec<UnsignedInt>>,
    pub classification: Coding,
    pub category: Option<Vec<Coding>>,
    pub control: Option<Vec<Coding>>,
}

/// Choice of types for the entity\[x\] field in ContractTermAssetValuedItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ContractTermAssetValuedItemEntity {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "entityCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "entityReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContractTermAssetValuedItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub entity: Option<ContractTermAssetValuedItemEntity>,
    pub identifier: Option<Identifier>,
    #[fhir_serde(rename = "effectiveTime")]
    pub effective_time: Option<DateTime>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub points: Option<Decimal>,
    pub net: Option<Money>,
    pub payment: Option<String>,
    #[fhir_serde(rename = "paymentDate")]
    pub payment_date: Option<DateTime>,
    pub responsible: Option<Reference>,
    pub recipient: Option<Reference>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<Vec<String>>,
    #[fhir_serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageCostToBeneficiaryException {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageClass {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub value: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Coverage {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "policyHolder")]
    pub policy_holder: Option<Reference>,
    pub subscriber: Option<Reference>,
    #[fhir_serde(rename = "subscriberId")]
    pub subscriber_id: Option<String>,
    pub beneficiary: Reference,
    pub dependent: Option<String>,
    pub relationship: Option<CodeableConcept>,
    pub period: Option<Period>,
    pub payor: Option<Vec<Reference>>,
    pub class: Option<Vec<CoverageClass>>,
    pub order: Option<PositiveInt>,
    pub network: Option<String>,
    #[fhir_serde(rename = "costToBeneficiary")]
    pub cost_to_beneficiary: Option<Vec<CoverageCostToBeneficiary>>,
    pub subrogation: Option<Boolean>,
    pub contract: Option<Vec<Reference>>,
}

/// Choice of types for the value\[x\] field in CoverageCostToBeneficiary
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CoverageCostToBeneficiaryValue {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "valueMoney")]
    Money(Money),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageCostToBeneficiary {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<CoverageCostToBeneficiaryValue>,
    pub exception: Option<Vec<CoverageCostToBeneficiaryException>>,
}


/// Choice of types for the diagnosis\[x\] field in CoverageEligibilityRequestItemDiagnosis
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CoverageEligibilityRequestItemDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityRequestItemDiagnosis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub diagnosis: Option<CoverageEligibilityRequestItemDiagnosisDiagnosis>,
}

/// Choice of types for the serviced\[x\] field in CoverageEligibilityRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CoverageEligibilityRequestServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityRequest {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub priority: Option<CodeableConcept>,
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    #[fhir_serde(flatten)]
    pub serviced: Option<CoverageEligibilityRequestServiced>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub provider: Option<Reference>,
    pub insurer: Reference,
    pub facility: Option<Reference>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<CoverageEligibilityRequestSupportingInfo>>,
    pub insurance: Option<Vec<CoverageEligibilityRequestInsurance>>,
    pub item: Option<Vec<CoverageEligibilityRequestItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityRequestItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "supportingInfoSequence")]
    pub supporting_info_sequence: Option<Vec<PositiveInt>>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub provider: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub facility: Option<Reference>,
    pub diagnosis: Option<Vec<CoverageEligibilityRequestItemDiagnosis>>,
    pub detail: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityRequestSupportingInfo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub information: Reference,
    #[fhir_serde(rename = "appliesToAll")]
    pub applies_to_all: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityRequestInsurance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub focal: Option<Boolean>,
    pub coverage: Reference,
    #[fhir_serde(rename = "businessArrangement")]
    pub business_arrangement: Option<String>,
}


/// Choice of types for the allowed\[x\] field in CoverageEligibilityResponseInsuranceItemBenefit
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CoverageEligibilityResponseInsuranceItemBenefitAllowed {
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "allowedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "allowedString")]
    String(String),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "allowedMoney")]
    Money(Money),
}

/// Choice of types for the used\[x\] field in CoverageEligibilityResponseInsuranceItemBenefit
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CoverageEligibilityResponseInsuranceItemBenefitUsed {
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "usedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "usedString")]
    String(String),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "usedMoney")]
    Money(Money),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityResponseInsuranceItemBenefit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub allowed: Option<CoverageEligibilityResponseInsuranceItemBenefitAllowed>,
    #[fhir_serde(flatten)]
    pub used: Option<CoverageEligibilityResponseInsuranceItemBenefitUsed>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityResponseInsuranceItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub provider: Option<Reference>,
    pub excluded: Option<Boolean>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub network: Option<CodeableConcept>,
    pub unit: Option<CodeableConcept>,
    pub term: Option<CodeableConcept>,
    pub benefit: Option<Vec<CoverageEligibilityResponseInsuranceItemBenefit>>,
    #[fhir_serde(rename = "authorizationRequired")]
    pub authorization_required: Option<Boolean>,
    #[fhir_serde(rename = "authorizationSupporting")]
    pub authorization_supporting: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "authorizationUrl")]
    pub authorization_url: Option<Uri>,
}

/// Choice of types for the serviced\[x\] field in CoverageEligibilityResponse
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum CoverageEligibilityResponseServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityResponse {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    #[fhir_serde(flatten)]
    pub serviced: Option<CoverageEligibilityResponseServiced>,
    pub created: DateTime,
    pub requestor: Option<Reference>,
    pub request: Reference,
    pub outcome: Code,
    pub disposition: Option<String>,
    pub insurer: Reference,
    pub insurance: Option<Vec<CoverageEligibilityResponseInsurance>>,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<String>,
    pub form: Option<CodeableConcept>,
    pub error: Option<Vec<CoverageEligibilityResponseError>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityResponseError {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityResponseInsurance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    pub inforce: Option<Boolean>,
    #[fhir_serde(rename = "benefitPeriod")]
    pub benefit_period: Option<Period>,
    pub item: Option<Vec<CoverageEligibilityResponseInsuranceItem>>,
}


/// Choice of types for the identified\[x\] field in DetectedIssue
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DetectedIssueIdentified {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "identifiedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "identifiedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DetectedIssue {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub code: Option<CodeableConcept>,
    pub severity: Option<Code>,
    pub patient: Option<Reference>,
    #[fhir_serde(flatten)]
    pub identified: Option<DetectedIssueIdentified>,
    pub author: Option<Reference>,
    pub implicated: Option<Vec<Reference>>,
    pub evidence: Option<Vec<DetectedIssueEvidence>>,
    pub detail: Option<String>,
    pub reference: Option<Uri>,
    pub mitigation: Option<Vec<DetectedIssueMitigation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DetectedIssueMitigation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: CodeableConcept,
    pub date: Option<DateTime>,
    pub author: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DetectedIssueEvidence {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub detail: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceVersion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub component: Option<Identifier>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Device {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub definition: Option<Reference>,
    #[fhir_serde(rename = "udiCarrier")]
    pub udi_carrier: Option<Vec<DeviceUdiCarrier>>,
    pub status: Option<Code>,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "distinctIdentifier")]
    pub distinct_identifier: Option<String>,
    pub manufacturer: Option<String>,
    #[fhir_serde(rename = "manufactureDate")]
    pub manufacture_date: Option<DateTime>,
    #[fhir_serde(rename = "expirationDate")]
    pub expiration_date: Option<DateTime>,
    #[fhir_serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    #[fhir_serde(rename = "serialNumber")]
    pub serial_number: Option<String>,
    #[fhir_serde(rename = "deviceName")]
    pub device_name: Option<Vec<DeviceDeviceName>>,
    #[fhir_serde(rename = "modelNumber")]
    pub model_number: Option<String>,
    #[fhir_serde(rename = "partNumber")]
    pub part_number: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub specialization: Option<Vec<DeviceSpecialization>>,
    pub version: Option<Vec<DeviceVersion>>,
    pub property: Option<Vec<DeviceProperty>>,
    pub patient: Option<Reference>,
    pub owner: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub location: Option<Reference>,
    pub url: Option<Uri>,
    pub note: Option<Vec<Annotation>>,
    pub safety: Option<Vec<CodeableConcept>>,
    pub parent: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceSpecialization {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "systemType")]
    pub system_type: CodeableConcept,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDeviceName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceUdiCarrier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "deviceIdentifier")]
    pub device_identifier: Option<String>,
    pub issuer: Option<Uri>,
    pub jurisdiction: Option<Uri>,
    #[fhir_serde(rename = "carrierAIDC")]
    pub carrier_a_i_d_c: Option<Base64Binary>,
    #[fhir_serde(rename = "carrierHRF")]
    pub carrier_h_r_f: Option<String>,
    #[fhir_serde(rename = "entryType")]
    pub entry_type: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(rename = "valueQuantity")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[fhir_serde(rename = "valueCode")]
    pub value_code: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionMaterial {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: CodeableConcept,
    pub alternate: Option<Boolean>,
    #[fhir_serde(rename = "allergenicIndicator")]
    pub allergenic_indicator: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionDeviceName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionSpecialization {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "systemType")]
    pub system_type: String,
    pub version: Option<String>,
}

/// Choice of types for the manufacturer\[x\] field in DeviceDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DeviceDefinitionManufacturer {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "manufacturerString")]
    String(String),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "manufacturerReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "udiDeviceIdentifier")]
    pub udi_device_identifier: Option<Vec<DeviceDefinitionUdiDeviceIdentifier>>,
    #[fhir_serde(flatten)]
    pub manufacturer: Option<DeviceDefinitionManufacturer>,
    #[fhir_serde(rename = "deviceName")]
    pub device_name: Option<Vec<DeviceDefinitionDeviceName>>,
    #[fhir_serde(rename = "modelNumber")]
    pub model_number: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub specialization: Option<Vec<DeviceDefinitionSpecialization>>,
    pub version: Option<Vec<String>>,
    pub safety: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "shelfLifeStorage")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    #[fhir_serde(rename = "physicalCharacteristics")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[fhir_serde(rename = "languageCode")]
    pub language_code: Option<Vec<CodeableConcept>>,
    pub capability: Option<Vec<DeviceDefinitionCapability>>,
    pub property: Option<Vec<DeviceDefinitionProperty>>,
    pub owner: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub url: Option<Uri>,
    #[fhir_serde(rename = "onlineInformation")]
    pub online_information: Option<Uri>,
    pub note: Option<Vec<Annotation>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "parentDevice")]
    pub parent_device: Option<Reference>,
    pub material: Option<Vec<DeviceDefinitionMaterial>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionCapability {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub description: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(rename = "valueQuantity")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[fhir_serde(rename = "valueCode")]
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionUdiDeviceIdentifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "deviceIdentifier")]
    pub device_identifier: String,
    pub issuer: Uri,
    pub jurisdiction: Uri,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceMetric {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub unit: Option<CodeableConcept>,
    pub source: Option<Reference>,
    pub parent: Option<Reference>,
    #[fhir_serde(rename = "operationalStatus")]
    pub operational_status: Option<Code>,
    pub color: Option<Code>,
    pub category: Code,
    #[fhir_serde(rename = "measurementPeriod")]
    pub measurement_period: Option<Timing>,
    pub calibration: Option<Vec<DeviceMetricCalibration>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceMetricCalibration {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub state: Option<Code>,
    pub time: Option<Instant>,
}


/// Choice of types for the code\[x\] field in DeviceRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DeviceRequestCode {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "codeReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "codeCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the occurrence\[x\] field in DeviceRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DeviceRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceRequest {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "priorRequest")]
    pub prior_request: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Option<Code>,
    pub intent: Code,
    pub priority: Option<Code>,
    #[fhir_serde(flatten)]
    pub code: Option<DeviceRequestCode>,
    pub parameter: Option<Vec<DeviceRequestParameter>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<DeviceRequestOccurrence>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    #[fhir_serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub performer: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub insurance: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
}

/// Choice of types for the value\[x\] field in DeviceRequestParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DeviceRequestParameterValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceRequestParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<DeviceRequestParameterValue>,
}


/// Choice of types for the timing\[x\] field in DeviceUseStatement
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DeviceUseStatementTiming {
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timingDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceUseStatement {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    pub subject: Reference,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub timing: Option<DeviceUseStatementTiming>,
    #[fhir_serde(rename = "recordedOn")]
    pub recorded_on: Option<DateTime>,
    pub source: Option<Reference>,
    pub device: Reference,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DiagnosticReportMedia {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub comment: Option<String>,
    pub link: Reference,
}

/// Choice of types for the effective\[x\] field in DiagnosticReport
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DiagnosticReportEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DiagnosticReport {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub effective: Option<DiagnosticReportEffective>,
    pub issued: Option<Instant>,
    pub performer: Option<Vec<Reference>>,
    #[fhir_serde(rename = "resultsInterpreter")]
    pub results_interpreter: Option<Vec<Reference>>,
    pub specimen: Option<Vec<Reference>>,
    pub result: Option<Vec<Reference>>,
    #[fhir_serde(rename = "imagingStudy")]
    pub imaging_study: Option<Vec<Reference>>,
    pub media: Option<Vec<DiagnosticReportMedia>>,
    pub conclusion: Option<String>,
    #[fhir_serde(rename = "conclusionCode")]
    pub conclusion_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "presentedForm")]
    pub presented_form: Option<Vec<Attachment>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentManifest {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "masterIdentifier")]
    pub master_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub created: Option<DateTime>,
    pub author: Option<Vec<Reference>>,
    pub recipient: Option<Vec<Reference>>,
    pub source: Option<Uri>,
    pub description: Option<String>,
    pub content: Option<Vec<Reference>>,
    pub related: Option<Vec<DocumentManifestRelated>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentManifestRelated {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[fhir_serde(rename = "ref")]
    pub r#ref: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentReferenceContext {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub encounter: Option<Vec<Reference>>,
    pub event: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    #[fhir_serde(rename = "facilityType")]
    pub facility_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "practiceSetting")]
    pub practice_setting: Option<CodeableConcept>,
    #[fhir_serde(rename = "sourcePatientInfo")]
    pub source_patient_info: Option<Reference>,
    pub related: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentReference {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "masterIdentifier")]
    pub master_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "docStatus")]
    pub doc_status: Option<Code>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub date: Option<Instant>,
    pub author: Option<Vec<Reference>>,
    pub authenticator: Option<Reference>,
    pub custodian: Option<Reference>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<DocumentReferenceRelatesTo>>,
    pub description: Option<String>,
    #[fhir_serde(rename = "securityLabel")]
    pub security_label: Option<Vec<CodeableConcept>>,
    pub content: Option<Vec<DocumentReferenceContent>>,
    pub context: Option<DocumentReferenceContext>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentReferenceRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub target: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentReferenceContent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub attachment: Attachment,
    pub format: Option<Coding>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Encounter {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "statusHistory")]
    pub status_history: Option<Vec<EncounterStatusHistory>>,
    pub class: Coding,
    #[fhir_serde(rename = "classHistory")]
    pub class_history: Option<Vec<EncounterClassHistory>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "serviceType")]
    pub service_type: Option<CodeableConcept>,
    pub priority: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    #[fhir_serde(rename = "episodeOfCare")]
    pub episode_of_care: Option<Vec<Reference>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub participant: Option<Vec<EncounterParticipant>>,
    pub appointment: Option<Vec<Reference>>,
    pub period: Option<Period>,
    pub length: Option<Duration>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub diagnosis: Option<Vec<EncounterDiagnosis>>,
    pub account: Option<Vec<Reference>>,
    pub hospitalization: Option<EncounterHospitalization>,
    pub location: Option<Vec<EncounterLocation>>,
    #[fhir_serde(rename = "serviceProvider")]
    pub service_provider: Option<Reference>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterLocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub location: Reference,
    pub status: Option<Code>,
    #[fhir_serde(rename = "physicalType")]
    pub physical_type: Option<CodeableConcept>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterStatusHistory {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterHospitalization {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "preAdmissionIdentifier")]
    pub pre_admission_identifier: Option<Identifier>,
    pub origin: Option<Reference>,
    #[fhir_serde(rename = "admitSource")]
    pub admit_source: Option<CodeableConcept>,
    #[fhir_serde(rename = "reAdmission")]
    pub re_admission: Option<CodeableConcept>,
    #[fhir_serde(rename = "dietPreference")]
    pub diet_preference: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "specialCourtesy")]
    pub special_courtesy: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "specialArrangement")]
    pub special_arrangement: Option<Vec<CodeableConcept>>,
    pub destination: Option<Reference>,
    #[fhir_serde(rename = "dischargeDisposition")]
    pub discharge_disposition: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterClassHistory {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub class: Coding,
    pub period: Period,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub individual: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterDiagnosis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<CodeableConcept>,
    pub rank: Option<PositiveInt>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Endpoint {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "connectionType")]
    pub connection_type: Coding,
    pub name: Option<String>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub period: Option<Period>,
    #[fhir_serde(rename = "payloadType")]
    pub payload_type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "payloadMimeType")]
    pub payload_mime_type: Option<Vec<Code>>,
    pub address: Url,
    pub header: Option<Vec<String>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EnrollmentRequest {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub created: Option<DateTime>,
    pub insurer: Option<Reference>,
    pub provider: Option<Reference>,
    pub candidate: Option<Reference>,
    pub coverage: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EnrollmentResponse {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub request: Option<Reference>,
    pub outcome: Option<Code>,
    pub disposition: Option<String>,
    pub created: Option<DateTime>,
    pub organization: Option<Reference>,
    #[fhir_serde(rename = "requestProvider")]
    pub request_provider: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EpisodeOfCare {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "statusHistory")]
    pub status_history: Option<Vec<EpisodeOfCareStatusHistory>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub diagnosis: Option<Vec<EpisodeOfCareDiagnosis>>,
    pub patient: Reference,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub period: Option<Period>,
    #[fhir_serde(rename = "referralRequest")]
    pub referral_request: Option<Vec<Reference>>,
    #[fhir_serde(rename = "careManager")]
    pub care_manager: Option<Reference>,
    pub team: Option<Vec<Reference>>,
    pub account: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EpisodeOfCareDiagnosis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    pub role: Option<CodeableConcept>,
    pub rank: Option<PositiveInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EpisodeOfCareStatusHistory {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}


/// Choice of types for the subject\[x\] field in EventDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum EventDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EventDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub subject: Option<EventDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub trigger: Option<Vec<TriggerDefinition>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceVariableDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "variableRole")]
    pub variable_role: CodeableConcept,
    pub observed: Option<Reference>,
    pub intended: Option<Reference>,
    #[fhir_serde(rename = "directnessMatch")]
    pub directness_match: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceCertainty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub rating: Option<CodeableConcept>,
    pub rater: Option<String>,
    pub subcomponent: Option<Vec<EvidenceCertainty>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceStatisticSampleSize {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "numberOfStudies")]
    pub number_of_studies: Option<UnsignedInt>,
    #[fhir_serde(rename = "numberOfParticipants")]
    pub number_of_participants: Option<UnsignedInt>,
    #[fhir_serde(rename = "knownDataCount")]
    pub known_data_count: Option<UnsignedInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceStatisticAttributeEstimate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    pub level: Option<Decimal>,
    pub range: Option<Range>,
    #[fhir_serde(rename = "attributeEstimate")]
    pub attribute_estimate: Option<Vec<EvidenceStatisticAttributeEstimate>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceStatisticModelCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: Option<Quantity>,
    pub variable: Option<Vec<EvidenceStatisticModelCharacteristicVariable>>,
    #[fhir_serde(rename = "attributeEstimate")]
    pub attribute_estimate: Option<Vec<EvidenceStatisticAttributeEstimate>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceStatisticModelCharacteristicVariable {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "variableDefinition")]
    pub variable_definition: Reference,
    pub handling: Option<Code>,
    #[fhir_serde(rename = "valueCategory")]
    pub value_category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "valueQuantity")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[fhir_serde(rename = "valueRange")]
    pub value_range: Option<Vec<Range>>,
}

/// Choice of types for the citeAs\[x\] field in Evidence
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum EvidenceCiteAs {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "citeAsReference")]
    Reference(Reference),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "citeAsMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Evidence {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(flatten)]
    pub cite_as: Option<EvidenceCiteAs>,
    pub status: Code,
    pub date: Option<DateTime>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub description: Option<Markdown>,
    pub assertion: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "variableDefinition")]
    pub variable_definition: Option<Vec<EvidenceVariableDefinition>>,
    #[fhir_serde(rename = "synthesisType")]
    pub synthesis_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "studyType")]
    pub study_type: Option<CodeableConcept>,
    pub statistic: Option<Vec<EvidenceStatistic>>,
    pub certainty: Option<Vec<EvidenceCertainty>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceStatistic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "statisticType")]
    pub statistic_type: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "numberOfEvents")]
    pub number_of_events: Option<UnsignedInt>,
    #[fhir_serde(rename = "numberAffected")]
    pub number_affected: Option<UnsignedInt>,
    #[fhir_serde(rename = "sampleSize")]
    pub sample_size: Option<EvidenceStatisticSampleSize>,
    #[fhir_serde(rename = "attributeEstimate")]
    pub attribute_estimate: Option<Vec<EvidenceStatisticAttributeEstimate>>,
    #[fhir_serde(rename = "modelCharacteristic")]
    pub model_characteristic: Option<Vec<EvidenceStatisticModelCharacteristic>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceReportSubject {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub characteristic: Option<Vec<EvidenceReportSubjectCharacteristic>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the target\[x\] field in EvidenceReportRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum EvidenceReportRelatesToTarget {
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "targetIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceReportRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[fhir_serde(flatten)]
    pub target: Option<EvidenceReportRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceReportSection {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub focus: Option<CodeableConcept>,
    #[fhir_serde(rename = "focusReference")]
    pub focus_reference: Option<Reference>,
    pub author: Option<Vec<Reference>>,
    pub text: Option<Narrative>,
    pub mode: Option<Code>,
    #[fhir_serde(rename = "orderedBy")]
    pub ordered_by: Option<CodeableConcept>,
    #[fhir_serde(rename = "entryClassifier")]
    pub entry_classifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "entryReference")]
    pub entry_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "entryQuantity")]
    pub entry_quantity: Option<Vec<Quantity>>,
    #[fhir_serde(rename = "emptyReason")]
    pub empty_reason: Option<CodeableConcept>,
    pub section: Option<Vec<EvidenceReportSection>>,
}

/// Choice of types for the citeAs\[x\] field in EvidenceReport
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum EvidenceReportCiteAs {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "citeAsReference")]
    Reference(Reference),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "citeAsMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceReport {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub status: Code,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "relatedIdentifier")]
    pub related_identifier: Option<Vec<Identifier>>,
    #[fhir_serde(flatten)]
    pub cite_as: Option<EvidenceReportCiteAs>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub subject: EvidenceReportSubject,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<EvidenceReportRelatesTo>>,
    pub section: Option<Vec<EvidenceReportSection>>,
}

/// Choice of types for the value\[x\] field in EvidenceReportSubjectCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum EvidenceReportSubjectCharacteristicValue {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceReportSubjectCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<EvidenceReportSubjectCharacteristicValue>,
    pub exclude: Option<Boolean>,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceVariable {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "shortTitle")]
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub date: Option<DateTime>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub actual: Option<Boolean>,
    #[fhir_serde(rename = "characteristicCombination")]
    pub characteristic_combination: Option<Code>,
    pub characteristic: Option<Vec<EvidenceVariableCharacteristic>>,
    pub handling: Option<Code>,
    pub category: Option<Vec<EvidenceVariableCategory>>,
}

/// Choice of types for the value\[x\] field in EvidenceVariableCategory
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum EvidenceVariableCategoryValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceVariableCategory {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    #[fhir_serde(flatten)]
    pub value: Option<EvidenceVariableCategoryValue>,
}

/// Choice of types for the definition\[x\] field in EvidenceVariableCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum EvidenceVariableCharacteristicDefinition {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "definitionReference")]
    Reference(Reference),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "definitionCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "definitionExpression")]
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceVariableCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    #[fhir_serde(flatten)]
    pub definition: Option<EvidenceVariableCharacteristicDefinition>,
    pub method: Option<CodeableConcept>,
    pub device: Option<Reference>,
    pub exclude: Option<Boolean>,
    #[fhir_serde(rename = "timeFromStart")]
    pub time_from_start: Option<EvidenceVariableCharacteristicTimeFromStart>,
    #[fhir_serde(rename = "groupMeasure")]
    pub group_measure: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceVariableCharacteristicTimeFromStart {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub quantity: Option<Quantity>,
    pub range: Option<Range>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioActor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "actorId")]
    pub actor_id: String,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub name: Option<String>,
    pub description: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioProcess {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "preConditions")]
    pub pre_conditions: Option<Markdown>,
    #[fhir_serde(rename = "postConditions")]
    pub post_conditions: Option<Markdown>,
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioProcessStepAlternative {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    pub description: Option<Markdown>,
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenario {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub purpose: Option<Markdown>,
    pub actor: Option<Vec<ExampleScenarioActor>>,
    pub instance: Option<Vec<ExampleScenarioInstance>>,
    pub process: Option<Vec<ExampleScenarioProcess>>,
    pub workflow: Option<Vec<Canonical>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioInstanceContainedInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "resourceId")]
    pub resource_id: String,
    #[fhir_serde(rename = "versionId")]
    pub version_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioInstanceVersion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "versionId")]
    pub version_id: String,
    pub description: Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioProcessStep {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub process: Option<Vec<ExampleScenarioProcess>>,
    pub pause: Option<Boolean>,
    pub operation: Option<ExampleScenarioProcessStepOperation>,
    pub alternative: Option<Vec<ExampleScenarioProcessStepAlternative>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioProcessStepOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: String,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<String>,
    pub name: Option<String>,
    pub initiator: Option<String>,
    pub receiver: Option<String>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "initiatorActive")]
    pub initiator_active: Option<Boolean>,
    #[fhir_serde(rename = "receiverActive")]
    pub receiver_active: Option<Boolean>,
    pub request: Option<ExampleScenarioInstanceContainedInstance>,
    pub response: Option<ExampleScenarioInstanceContainedInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "resourceId")]
    pub resource_id: String,
    #[fhir_serde(rename = "resourceType")]
    pub resource_type: Code,
    pub name: Option<String>,
    pub description: Option<Markdown>,
    pub version: Option<Vec<ExampleScenarioInstanceVersion>>,
    #[fhir_serde(rename = "containedInstance")]
    pub contained_instance: Option<Vec<ExampleScenarioInstanceContainedInstance>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitInsurance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub focal: Boolean,
    pub coverage: Reference,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<Vec<String>>,
}

/// Choice of types for the procedure\[x\] field in ExplanationOfBenefitProcedure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitProcedureProcedure {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "procedureReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitProcedure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub date: Option<DateTime>,
    #[fhir_serde(flatten)]
    pub procedure: Option<ExplanationOfBenefitProcedureProcedure>,
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitAddItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitAddItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitProcessNote {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<PositiveInt>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: Option<String>,
    pub language: Option<CodeableConcept>,
}

/// Choice of types for the serviced\[x\] field in ExplanationOfBenefitItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitItemServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location\[x\] field in ExplanationOfBenefitItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitItemLocation {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "careTeamSequence")]
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "diagnosisSequence")]
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "procedureSequence")]
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "informationSequence")]
    pub information_sequence: Option<Vec<PositiveInt>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub serviced: Option<ExplanationOfBenefitItemServiced>,
    #[fhir_serde(flatten)]
    pub location: Option<ExplanationOfBenefitItemLocation>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[fhir_serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    pub encounter: Option<Vec<Reference>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub detail: Option<Vec<ExplanationOfBenefitItemDetail>>,
}

/// Choice of types for the diagnosis\[x\] field in ExplanationOfBenefitDiagnosis
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitDiagnosis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(flatten)]
    pub diagnosis: Option<ExplanationOfBenefitDiagnosisDiagnosis>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "onAdmission")]
    pub on_admission: Option<CodeableConcept>,
    #[fhir_serde(rename = "packageCode")]
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitCareTeam {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub provider: Reference,
    pub responsible: Option<Boolean>,
    pub role: Option<CodeableConcept>,
    pub qualification: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitItemAdjudication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub reason: Option<CodeableConcept>,
    pub amount: Option<Money>,
    pub value: Option<Decimal>,
}

/// Choice of types for the timing\[x\] field in ExplanationOfBenefitSupportingInfo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitSupportingInfoTiming {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timingPeriod")]
    Period(Period),
}

/// Choice of types for the value\[x\] field in ExplanationOfBenefitSupportingInfo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitSupportingInfoValue {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitSupportingInfo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub timing: Option<ExplanationOfBenefitSupportingInfoTiming>,
    #[fhir_serde(flatten)]
    pub value: Option<ExplanationOfBenefitSupportingInfoValue>,
    pub reason: Option<Coding>,
}

/// Choice of types for the serviced\[x\] field in ExplanationOfBenefitAddItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitAddItemServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location\[x\] field in ExplanationOfBenefitAddItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitAddItemLocation {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitAddItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "itemSequence")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "detailSequence")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: Option<Vec<PositiveInt>>,
    pub provider: Option<Vec<Reference>>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub serviced: Option<ExplanationOfBenefitAddItemServiced>,
    #[fhir_serde(flatten)]
    pub location: Option<ExplanationOfBenefitAddItemLocation>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[fhir_serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub detail: Option<Vec<ExplanationOfBenefitAddItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitTotal {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitAddItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

/// Choice of types for the allowed\[x\] field in ExplanationOfBenefitBenefitBalanceFinancial
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitBenefitBalanceFinancialAllowed {
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "allowedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "allowedString")]
    String(String),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "allowedMoney")]
    Money(Money),
}

/// Choice of types for the used\[x\] field in ExplanationOfBenefitBenefitBalanceFinancial
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitBenefitBalanceFinancialUsed {
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "usedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "usedMoney")]
    Money(Money),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitBenefitBalanceFinancial {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub allowed: Option<ExplanationOfBenefitBenefitBalanceFinancialAllowed>,
    #[fhir_serde(flatten)]
    pub used: Option<ExplanationOfBenefitBenefitBalanceFinancialUsed>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitPayee {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub party: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitPayment {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub adjustment: Option<Money>,
    #[fhir_serde(rename = "adjustmentReason")]
    pub adjustment_reason: Option<CodeableConcept>,
    pub date: Option<Date>,
    pub amount: Option<Money>,
    pub identifier: Option<Identifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefit {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    #[fhir_serde(rename = "billablePeriod")]
    pub billable_period: Option<Period>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub insurer: Reference,
    pub provider: Reference,
    pub priority: Option<CodeableConcept>,
    #[fhir_serde(rename = "fundsReserveRequested")]
    pub funds_reserve_requested: Option<CodeableConcept>,
    #[fhir_serde(rename = "fundsReserve")]
    pub funds_reserve: Option<CodeableConcept>,
    pub related: Option<Vec<ExplanationOfBenefitRelated>>,
    pub prescription: Option<Reference>,
    #[fhir_serde(rename = "originalPrescription")]
    pub original_prescription: Option<Reference>,
    pub payee: Option<ExplanationOfBenefitPayee>,
    pub referral: Option<Reference>,
    pub facility: Option<Reference>,
    pub claim: Option<Reference>,
    #[fhir_serde(rename = "claimResponse")]
    pub claim_response: Option<Reference>,
    pub outcome: Code,
    pub disposition: Option<String>,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[fhir_serde(rename = "preAuthRefPeriod")]
    pub pre_auth_ref_period: Option<Vec<Period>>,
    #[fhir_serde(rename = "careTeam")]
    pub care_team: Option<Vec<ExplanationOfBenefitCareTeam>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<ExplanationOfBenefitSupportingInfo>>,
    pub diagnosis: Option<Vec<ExplanationOfBenefitDiagnosis>>,
    pub procedure: Option<Vec<ExplanationOfBenefitProcedure>>,
    pub precedence: Option<PositiveInt>,
    pub insurance: Option<Vec<ExplanationOfBenefitInsurance>>,
    pub accident: Option<ExplanationOfBenefitAccident>,
    pub item: Option<Vec<ExplanationOfBenefitItem>>,
    #[fhir_serde(rename = "addItem")]
    pub add_item: Option<Vec<ExplanationOfBenefitAddItem>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub total: Option<Vec<ExplanationOfBenefitTotal>>,
    pub payment: Option<ExplanationOfBenefitPayment>,
    #[fhir_serde(rename = "formCode")]
    pub form_code: Option<CodeableConcept>,
    pub form: Option<Attachment>,
    #[fhir_serde(rename = "processNote")]
    pub process_note: Option<Vec<ExplanationOfBenefitProcessNote>>,
    #[fhir_serde(rename = "benefitPeriod")]
    pub benefit_period: Option<Period>,
    #[fhir_serde(rename = "benefitBalance")]
    pub benefit_balance: Option<Vec<ExplanationOfBenefitBenefitBalance>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitRelated {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub claim: Option<Reference>,
    pub relationship: Option<CodeableConcept>,
    pub reference: Option<Identifier>,
}

/// Choice of types for the location\[x\] field in ExplanationOfBenefitAccident
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExplanationOfBenefitAccidentLocation {
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitAccident {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<Date>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub location: Option<ExplanationOfBenefitAccidentLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitBenefitBalance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub excluded: Option<Boolean>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub network: Option<CodeableConcept>,
    pub unit: Option<CodeableConcept>,
    pub term: Option<CodeableConcept>,
    pub financial: Option<Vec<ExplanationOfBenefitBenefitBalanceFinancial>>,
}


/// Choice of types for the born\[x\] field in FamilyMemberHistory
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum FamilyMemberHistoryBorn {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "bornPeriod")]
    Period(Period),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "bornDate")]
    Date(Date),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "bornString")]
    String(String),
}

/// Choice of types for the age\[x\] field in FamilyMemberHistory
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum FamilyMemberHistoryAge {
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "ageAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "ageRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "ageString")]
    String(String),
}

/// Choice of types for the deceased\[x\] field in FamilyMemberHistory
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum FamilyMemberHistoryDeceased {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "deceasedAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "deceasedRange")]
    Range(Range),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "deceasedDate")]
    Date(Date),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "deceasedString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct FamilyMemberHistory {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    pub status: Code,
    #[fhir_serde(rename = "dataAbsentReason")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub patient: Reference,
    pub date: Option<DateTime>,
    pub name: Option<String>,
    pub relationship: CodeableConcept,
    pub sex: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub born: Option<FamilyMemberHistoryBorn>,
    #[fhir_serde(flatten)]
    pub age: Option<FamilyMemberHistoryAge>,
    #[fhir_serde(rename = "estimatedAge")]
    pub estimated_age: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub deceased: Option<FamilyMemberHistoryDeceased>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub condition: Option<Vec<FamilyMemberHistoryCondition>>,
}

/// Choice of types for the onset\[x\] field in FamilyMemberHistoryCondition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum FamilyMemberHistoryConditionOnset {
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "onsetString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct FamilyMemberHistoryCondition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub outcome: Option<CodeableConcept>,
    #[fhir_serde(rename = "contributedToDeath")]
    pub contributed_to_death: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub onset: Option<FamilyMemberHistoryConditionOnset>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Flag {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    pub period: Option<Period>,
    pub encounter: Option<Reference>,
    pub author: Option<Reference>,
}


/// Choice of types for the detail\[x\] field in GoalTarget
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum GoalTargetDetail {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "detailQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "detailRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "detailCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "detailString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "detailBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "detailInteger")]
    Integer(Integer),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "detailRatio")]
    Ratio(Ratio),
}

/// Choice of types for the due\[x\] field in GoalTarget
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum GoalTargetDue {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "dueDate")]
    Date(Date),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "dueDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GoalTarget {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub measure: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub detail: Option<GoalTargetDetail>,
    #[fhir_serde(flatten)]
    pub due: Option<GoalTargetDue>,
}

/// Choice of types for the start\[x\] field in Goal
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum GoalStart {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "startDate")]
    Date(Date),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "startCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Goal {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "lifecycleStatus")]
    pub lifecycle_status: Code,
    #[fhir_serde(rename = "achievementStatus")]
    pub achievement_status: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub subject: Reference,
    #[fhir_serde(flatten)]
    pub start: Option<GoalStart>,
    pub target: Option<Vec<GoalTarget>>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<Date>,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<String>,
    #[fhir_serde(rename = "expressedBy")]
    pub expressed_by: Option<Reference>,
    pub addresses: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "outcomeCode")]
    pub outcome_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "outcomeReference")]
    pub outcome_reference: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GraphDefinitionLinkTargetCompartment {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub code: Code,
    pub rule: Code,
    pub expression: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GraphDefinitionLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    #[fhir_serde(rename = "sliceName")]
    pub slice_name: Option<String>,
    pub min: Option<Integer>,
    pub max: Option<String>,
    pub description: Option<String>,
    pub target: Option<Vec<GraphDefinitionLinkTarget>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GraphDefinitionLinkTarget {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub params: Option<String>,
    pub profile: Option<Canonical>,
    pub compartment: Option<Vec<GraphDefinitionLinkTargetCompartment>>,
    pub link: Option<Vec<GraphDefinitionLink>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GraphDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub name: String,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub start: Code,
    pub profile: Option<Canonical>,
    pub link: Option<Vec<GraphDefinitionLink>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Group {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub actual: Boolean,
    pub code: Option<CodeableConcept>,
    pub name: Option<String>,
    pub quantity: Option<UnsignedInt>,
    #[fhir_serde(rename = "managingEntity")]
    pub managing_entity: Option<Reference>,
    pub characteristic: Option<Vec<GroupCharacteristic>>,
    pub member: Option<Vec<GroupMember>>,
}

/// Choice of types for the value\[x\] field in GroupCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum GroupCharacteristicValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GroupCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<GroupCharacteristicValue>,
    pub exclude: Boolean,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GroupMember {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub entity: Reference,
    pub period: Option<Period>,
    pub inactive: Option<Boolean>,
}


/// Choice of types for the module\[x\] field in GuidanceResponse
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum GuidanceResponseModule {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "moduleUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "moduleCanonical")]
    Canonical(Canonical),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "moduleCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GuidanceResponse {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "requestIdentifier")]
    pub request_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(flatten)]
    pub module: Option<GuidanceResponseModule>,
    pub status: Code,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    pub performer: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "evaluationMessage")]
    pub evaluation_message: Option<Vec<Reference>>,
    #[fhir_serde(rename = "outputParameters")]
    pub output_parameters: Option<Reference>,
    pub result: Option<Reference>,
    #[fhir_serde(rename = "dataRequirement")]
    pub data_requirement: Option<Vec<DataRequirement>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct HealthcareServiceEligibility {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub comment: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct HealthcareServiceNotAvailable {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub during: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct HealthcareService {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    #[fhir_serde(rename = "providedBy")]
    pub provided_by: Option<Reference>,
    pub category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    pub name: Option<String>,
    pub comment: Option<String>,
    #[fhir_serde(rename = "extraDetails")]
    pub extra_details: Option<Markdown>,
    pub photo: Option<Attachment>,
    pub telecom: Option<Vec<ContactPoint>>,
    #[fhir_serde(rename = "coverageArea")]
    pub coverage_area: Option<Vec<Reference>>,
    #[fhir_serde(rename = "serviceProvisionCode")]
    pub service_provision_code: Option<Vec<CodeableConcept>>,
    pub eligibility: Option<Vec<HealthcareServiceEligibility>>,
    pub program: Option<Vec<CodeableConcept>>,
    pub characteristic: Option<Vec<CodeableConcept>>,
    pub communication: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "referralMethod")]
    pub referral_method: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "appointmentRequired")]
    pub appointment_required: Option<Boolean>,
    #[fhir_serde(rename = "availableTime")]
    pub available_time: Option<Vec<HealthcareServiceAvailableTime>>,
    #[fhir_serde(rename = "notAvailable")]
    pub not_available: Option<Vec<HealthcareServiceNotAvailable>>,
    #[fhir_serde(rename = "availabilityExceptions")]
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct HealthcareServiceAvailableTime {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "daysOfWeek")]
    pub days_of_week: Option<Vec<Code>>,
    #[fhir_serde(rename = "allDay")]
    pub all_day: Option<Boolean>,
    #[fhir_serde(rename = "availableStartTime")]
    pub available_start_time: Option<Time>,
    #[fhir_serde(rename = "availableEndTime")]
    pub available_end_time: Option<Time>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingStudySeries {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    pub number: Option<UnsignedInt>,
    pub modality: Coding,
    pub description: Option<String>,
    #[fhir_serde(rename = "numberOfInstances")]
    pub number_of_instances: Option<UnsignedInt>,
    pub endpoint: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Coding>,
    pub laterality: Option<Coding>,
    pub specimen: Option<Vec<Reference>>,
    pub started: Option<DateTime>,
    pub performer: Option<Vec<ImagingStudySeriesPerformer>>,
    pub instance: Option<Vec<ImagingStudySeriesInstance>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingStudySeriesInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    #[fhir_serde(rename = "sopClass")]
    pub sop_class: Coding,
    pub number: Option<UnsignedInt>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingStudy {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub modality: Option<Vec<Coding>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub started: Option<DateTime>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub referrer: Option<Reference>,
    pub interpreter: Option<Vec<Reference>>,
    pub endpoint: Option<Vec<Reference>>,
    #[fhir_serde(rename = "numberOfSeries")]
    pub number_of_series: Option<UnsignedInt>,
    #[fhir_serde(rename = "numberOfInstances")]
    pub number_of_instances: Option<UnsignedInt>,
    #[fhir_serde(rename = "procedureReference")]
    pub procedure_reference: Option<Reference>,
    #[fhir_serde(rename = "procedureCode")]
    pub procedure_code: Option<Vec<CodeableConcept>>,
    pub location: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub description: Option<String>,
    pub series: Option<Vec<ImagingStudySeries>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingStudySeriesPerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationPerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationEducation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "documentType")]
    pub document_type: Option<String>,
    pub reference: Option<Uri>,
    #[fhir_serde(rename = "publicationDate")]
    pub publication_date: Option<DateTime>,
    #[fhir_serde(rename = "presentationDate")]
    pub presentation_date: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationReaction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<DateTime>,
    pub detail: Option<Reference>,
    pub reported: Option<Boolean>,
}

/// Choice of types for the occurrence\[x\] field in Immunization
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImmunizationOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "occurrenceString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Immunization {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    #[fhir_serde(rename = "vaccineCode")]
    pub vaccine_code: CodeableConcept,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<ImmunizationOccurrence>,
    pub recorded: Option<DateTime>,
    #[fhir_serde(rename = "primarySource")]
    pub primary_source: Option<Boolean>,
    #[fhir_serde(rename = "reportOrigin")]
    pub report_origin: Option<CodeableConcept>,
    pub location: Option<Reference>,
    pub manufacturer: Option<Reference>,
    #[fhir_serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    #[fhir_serde(rename = "expirationDate")]
    pub expiration_date: Option<Date>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    #[fhir_serde(rename = "doseQuantity")]
    pub dose_quantity: Option<Quantity>,
    pub performer: Option<Vec<ImmunizationPerformer>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "isSubpotent")]
    pub is_subpotent: Option<Boolean>,
    #[fhir_serde(rename = "subpotentReason")]
    pub subpotent_reason: Option<Vec<CodeableConcept>>,
    pub education: Option<Vec<ImmunizationEducation>>,
    #[fhir_serde(rename = "programEligibility")]
    pub program_eligibility: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "fundingSource")]
    pub funding_source: Option<CodeableConcept>,
    pub reaction: Option<Vec<ImmunizationReaction>>,
    #[fhir_serde(rename = "protocolApplied")]
    pub protocol_applied: Option<Vec<ImmunizationProtocolApplied>>,
}

/// Choice of types for the doseNumber\[x\] field in ImmunizationProtocolApplied
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImmunizationProtocolAppliedDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses\[x\] field in ImmunizationProtocolApplied
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImmunizationProtocolAppliedSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationProtocolApplied {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub series: Option<String>,
    pub authority: Option<Reference>,
    #[fhir_serde(rename = "targetDisease")]
    pub target_disease: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub dose_number: Option<ImmunizationProtocolAppliedDoseNumber>,
    #[fhir_serde(flatten)]
    pub series_doses: Option<ImmunizationProtocolAppliedSeriesDoses>,
}


/// Choice of types for the doseNumber\[x\] field in ImmunizationEvaluation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImmunizationEvaluationDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses\[x\] field in ImmunizationEvaluation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImmunizationEvaluationSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationEvaluation {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub patient: Reference,
    pub date: Option<DateTime>,
    pub authority: Option<Reference>,
    #[fhir_serde(rename = "targetDisease")]
    pub target_disease: CodeableConcept,
    #[fhir_serde(rename = "immunizationEvent")]
    pub immunization_event: Reference,
    #[fhir_serde(rename = "doseStatus")]
    pub dose_status: CodeableConcept,
    #[fhir_serde(rename = "doseStatusReason")]
    pub dose_status_reason: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub series: Option<String>,
    #[fhir_serde(flatten)]
    pub dose_number: Option<ImmunizationEvaluationDoseNumber>,
    #[fhir_serde(flatten)]
    pub series_doses: Option<ImmunizationEvaluationSeriesDoses>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationRecommendationRecommendationDateCriterion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: DateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationRecommendation {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub patient: Reference,
    pub date: DateTime,
    pub authority: Option<Reference>,
    pub recommendation: Option<Vec<ImmunizationRecommendationRecommendation>>,
}

/// Choice of types for the doseNumber\[x\] field in ImmunizationRecommendationRecommendation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImmunizationRecommendationRecommendationDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses\[x\] field in ImmunizationRecommendationRecommendation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImmunizationRecommendationRecommendationSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationRecommendationRecommendation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "vaccineCode")]
    pub vaccine_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "targetDisease")]
    pub target_disease: Option<CodeableConcept>,
    #[fhir_serde(rename = "contraindicatedVaccineCode")]
    pub contraindicated_vaccine_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "forecastStatus")]
    pub forecast_status: CodeableConcept,
    #[fhir_serde(rename = "forecastReason")]
    pub forecast_reason: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "dateCriterion")]
    pub date_criterion: Option<Vec<ImmunizationRecommendationRecommendationDateCriterion>>,
    pub description: Option<String>,
    pub series: Option<String>,
    #[fhir_serde(flatten)]
    pub dose_number: Option<ImmunizationRecommendationRecommendationDoseNumber>,
    #[fhir_serde(flatten)]
    pub series_doses: Option<ImmunizationRecommendationRecommendationSeriesDoses>,
    #[fhir_serde(rename = "supportingImmunization")]
    pub supporting_immunization: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingPatientInformation")]
    pub supporting_patient_information: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideManifest {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub rendering: Option<Url>,
    pub resource: Option<Vec<ImplementationGuideManifestResource>>,
    pub page: Option<Vec<ImplementationGuideManifestPage>>,
    pub image: Option<Vec<String>>,
    pub other: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDependsOn {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Canonical,
    #[fhir_serde(rename = "packageId")]
    pub package_id: Option<Id>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuide {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "packageId")]
    pub package_id: Id,
    pub license: Option<Code>,
    #[fhir_serde(rename = "fhirVersion")]
    pub fhir_version: Option<Vec<Code>>,
    #[fhir_serde(rename = "dependsOn")]
    pub depends_on: Option<Vec<ImplementationGuideDependsOn>>,
    pub global: Option<Vec<ImplementationGuideGlobal>>,
    pub definition: Option<ImplementationGuideDefinition>,
    pub manifest: Option<ImplementationGuideManifest>,
}

/// Choice of types for the name\[x\] field in ImplementationGuideDefinitionPage
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImplementationGuideDefinitionPageName {
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "nameUrl")]
    Url(Url),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "nameReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDefinitionPage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub name: Option<ImplementationGuideDefinitionPageName>,
    pub title: String,
    pub generation: Code,
    pub page: Option<Vec<ImplementationGuideDefinitionPage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDefinitionTemplate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub source: String,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDefinitionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDefinitionGrouping {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub grouping: Option<Vec<ImplementationGuideDefinitionGrouping>>,
    pub resource: Option<Vec<ImplementationGuideDefinitionResource>>,
    pub page: Option<ImplementationGuideDefinitionPage>,
    pub parameter: Option<Vec<ImplementationGuideDefinitionParameter>>,
    pub template: Option<Vec<ImplementationGuideDefinitionTemplate>>,
}

/// Choice of types for the example\[x\] field in ImplementationGuideDefinitionResource
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImplementationGuideDefinitionResourceExample {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "exampleBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "exampleCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDefinitionResource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[fhir_serde(rename = "fhirVersion")]
    pub fhir_version: Option<Vec<Code>>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[fhir_serde(flatten)]
    pub example: Option<ImplementationGuideDefinitionResourceExample>,
    #[fhir_serde(rename = "groupingId")]
    pub grouping_id: Option<Id>,
}

/// Choice of types for the example\[x\] field in ImplementationGuideManifestResource
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ImplementationGuideManifestResourceExample {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "exampleBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "exampleCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideManifestResource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[fhir_serde(flatten)]
    pub example: Option<ImplementationGuideManifestResourceExample>,
    #[fhir_serde(rename = "relativePath")]
    pub relative_path: Option<Url>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideGlobal {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub profile: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideManifestPage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub title: Option<String>,
    pub anchor: Option<Vec<String>>,
}


/// Choice of types for the presentation\[x\] field in IngredientSubstanceStrength
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum IngredientSubstanceStrengthPresentation {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "presentationRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "presentationRatioRange")]
    RatioRange(RatioRange),
}

/// Choice of types for the concentration\[x\] field in IngredientSubstanceStrength
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum IngredientSubstanceStrengthConcentration {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "concentrationRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "concentrationRatioRange")]
    RatioRange(RatioRange),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct IngredientSubstanceStrength {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub presentation: Option<IngredientSubstanceStrengthPresentation>,
    #[fhir_serde(rename = "textPresentation")]
    pub text_presentation: Option<String>,
    #[fhir_serde(flatten)]
    pub concentration: Option<IngredientSubstanceStrengthConcentration>,
    #[fhir_serde(rename = "textConcentration")]
    pub text_concentration: Option<String>,
    #[fhir_serde(rename = "measurementPoint")]
    pub measurement_point: Option<String>,
    pub country: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "referenceStrength")]
    pub reference_strength: Option<Vec<IngredientSubstanceStrengthReferenceStrength>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct IngredientSubstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableReference,
    pub strength: Option<Vec<IngredientSubstanceStrength>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct IngredientManufacturer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<Code>,
    pub manufacturer: Reference,
}

/// Choice of types for the strength\[x\] field in IngredientSubstanceStrengthReferenceStrength
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum IngredientSubstanceStrengthReferenceStrengthStrength {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "strengthRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "strengthRatioRange")]
    RatioRange(RatioRange),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct IngredientSubstanceStrengthReferenceStrength {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: Option<CodeableReference>,
    #[fhir_serde(flatten)]
    pub strength: Option<IngredientSubstanceStrengthReferenceStrengthStrength>,
    #[fhir_serde(rename = "measurementPoint")]
    pub measurement_point: Option<String>,
    pub country: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Ingredient {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub status: Code,
    #[fhir_serde(rename = "for")]
    pub r#for: Option<Vec<Reference>>,
    pub role: CodeableConcept,
    pub function: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "allergenicIndicator")]
    pub allergenic_indicator: Option<Boolean>,
    pub manufacturer: Option<Vec<IngredientManufacturer>>,
    pub substance: IngredientSubstance,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanPlan {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "coverageArea")]
    pub coverage_area: Option<Vec<Reference>>,
    pub network: Option<Vec<Reference>>,
    #[fhir_serde(rename = "generalCost")]
    pub general_cost: Option<Vec<InsurancePlanPlanGeneralCost>>,
    #[fhir_serde(rename = "specificCost")]
    pub specific_cost: Option<Vec<InsurancePlanPlanSpecificCost>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanPlanGeneralCost {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "groupSize")]
    pub group_size: Option<PositiveInt>,
    pub cost: Option<Money>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanCoverageBenefitLimit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<Quantity>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanPlanSpecificCostBenefitCost {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub applicability: Option<CodeableConcept>,
    pub qualifiers: Option<Vec<CodeableConcept>>,
    pub value: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanPlanSpecificCostBenefit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub cost: Option<Vec<InsurancePlanPlanSpecificCostBenefitCost>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanContact {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub purpose: Option<CodeableConcept>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlan {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub period: Option<Period>,
    #[fhir_serde(rename = "ownedBy")]
    pub owned_by: Option<Reference>,
    #[fhir_serde(rename = "administeredBy")]
    pub administered_by: Option<Reference>,
    #[fhir_serde(rename = "coverageArea")]
    pub coverage_area: Option<Vec<Reference>>,
    pub contact: Option<Vec<InsurancePlanContact>>,
    pub endpoint: Option<Vec<Reference>>,
    pub network: Option<Vec<Reference>>,
    pub coverage: Option<Vec<InsurancePlanCoverage>>,
    pub plan: Option<Vec<InsurancePlanPlan>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanCoverage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub network: Option<Vec<Reference>>,
    pub benefit: Option<Vec<InsurancePlanCoverageBenefit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanPlanSpecificCost {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub benefit: Option<Vec<InsurancePlanPlanSpecificCostBenefit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanCoverageBenefit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub requirement: Option<String>,
    pub limit: Option<Vec<InsurancePlanCoverageBenefitLimit>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InvoiceParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub actor: Reference,
}

/// Choice of types for the chargeItem\[x\] field in InvoiceLineItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum InvoiceLineItemChargeItem {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "chargeItemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "chargeItemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InvoiceLineItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<PositiveInt>,
    #[fhir_serde(flatten)]
    pub charge_item: Option<InvoiceLineItemChargeItem>,
    #[fhir_serde(rename = "priceComponent")]
    pub price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Invoice {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "cancelledReason")]
    pub cancelled_reason: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub recipient: Option<Reference>,
    pub date: Option<DateTime>,
    pub participant: Option<Vec<InvoiceParticipant>>,
    pub issuer: Option<Reference>,
    pub account: Option<Reference>,
    #[fhir_serde(rename = "lineItem")]
    pub line_item: Option<Vec<InvoiceLineItem>>,
    #[fhir_serde(rename = "totalPriceComponent")]
    pub total_price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
    #[fhir_serde(rename = "totalNet")]
    pub total_net: Option<Money>,
    #[fhir_serde(rename = "totalGross")]
    pub total_gross: Option<Money>,
    #[fhir_serde(rename = "paymentTerms")]
    pub payment_terms: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InvoiceLineItemPriceComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub code: Option<CodeableConcept>,
    pub factor: Option<Decimal>,
    pub amount: Option<Money>,
}


/// Choice of types for the subject\[x\] field in Library
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum LibrarySubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Library {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub subject: Option<LibrarySubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub parameter: Option<Vec<ParameterDefinition>>,
    #[fhir_serde(rename = "dataRequirement")]
    pub data_requirement: Option<Vec<DataRequirement>>,
    pub content: Option<Vec<Attachment>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct LinkageItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub resource: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Linkage {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub active: Option<Boolean>,
    pub author: Option<Reference>,
    pub item: Option<Vec<LinkageItem>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct List {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub mode: Code,
    pub title: Option<String>,
    pub code: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub date: Option<DateTime>,
    pub source: Option<Reference>,
    #[fhir_serde(rename = "orderedBy")]
    pub ordered_by: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
    pub entry: Option<Vec<ListEntry>>,
    #[fhir_serde(rename = "emptyReason")]
    pub empty_reason: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ListEntry {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub flag: Option<CodeableConcept>,
    pub deleted: Option<Boolean>,
    pub date: Option<DateTime>,
    pub item: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Location {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    #[fhir_serde(rename = "operationalStatus")]
    pub operational_status: Option<Coding>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub description: Option<String>,
    pub mode: Option<Code>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
    #[fhir_serde(rename = "physicalType")]
    pub physical_type: Option<CodeableConcept>,
    pub position: Option<LocationPosition>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Reference>,
    #[fhir_serde(rename = "hoursOfOperation")]
    pub hours_of_operation: Option<Vec<LocationHoursOfOperation>>,
    #[fhir_serde(rename = "availabilityExceptions")]
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct LocationPosition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub longitude: Decimal,
    pub latitude: Decimal,
    pub altitude: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct LocationHoursOfOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "daysOfWeek")]
    pub days_of_week: Option<Vec<Code>>,
    #[fhir_serde(rename = "allDay")]
    pub all_day: Option<Boolean>,
    #[fhir_serde(rename = "openingTime")]
    pub opening_time: Option<Time>,
    #[fhir_serde(rename = "closingTime")]
    pub closing_time: Option<Time>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ManufacturedItemDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "manufacturedDoseForm")]
    pub manufactured_dose_form: CodeableConcept,
    #[fhir_serde(rename = "unitOfPresentation")]
    pub unit_of_presentation: Option<CodeableConcept>,
    pub manufacturer: Option<Vec<Reference>>,
    pub ingredient: Option<Vec<CodeableConcept>>,
    pub property: Option<Vec<ManufacturedItemDefinitionProperty>>,
}

/// Choice of types for the value\[x\] field in ManufacturedItemDefinitionProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ManufacturedItemDefinitionPropertyValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ManufacturedItemDefinitionProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<ManufacturedItemDefinitionPropertyValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub population: Option<Vec<MeasureGroupPopulation>>,
    pub stratifier: Option<Vec<MeasureGroupStratifier>>,
}

/// Choice of types for the subject\[x\] field in Measure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MeasureSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Measure {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub subject: Option<MeasureSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub disclaimer: Option<Markdown>,
    pub scoring: Option<CodeableConcept>,
    #[fhir_serde(rename = "compositeScoring")]
    pub composite_scoring: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "riskAdjustment")]
    pub risk_adjustment: Option<String>,
    #[fhir_serde(rename = "rateAggregation")]
    pub rate_aggregation: Option<String>,
    pub rationale: Option<Markdown>,
    #[fhir_serde(rename = "clinicalRecommendationStatement")]
    pub clinical_recommendation_statement: Option<Markdown>,
    #[fhir_serde(rename = "improvementNotation")]
    pub improvement_notation: Option<CodeableConcept>,
    pub definition: Option<Vec<Markdown>>,
    pub guidance: Option<Markdown>,
    pub group: Option<Vec<MeasureGroup>>,
    #[fhir_serde(rename = "supplementalData")]
    pub supplemental_data: Option<Vec<MeasureSupplementalData>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureGroupStratifierComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureSupplementalData {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub usage: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureGroupPopulation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureGroupStratifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Option<Expression>,
    pub component: Option<Vec<MeasureGroupStratifierComponent>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroupStratifierStratumPopulation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub count: Option<Integer>,
    #[fhir_serde(rename = "subjectResults")]
    pub subject_results: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub population: Option<Vec<MeasureReportGroupPopulation>>,
    #[fhir_serde(rename = "measureScore")]
    pub measure_score: Option<Quantity>,
    pub stratifier: Option<Vec<MeasureReportGroupStratifier>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroupStratifierStratum {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<CodeableConcept>,
    pub component: Option<Vec<MeasureReportGroupStratifierStratumComponent>>,
    pub population: Option<Vec<MeasureReportGroupStratifierStratumPopulation>>,
    #[fhir_serde(rename = "measureScore")]
    pub measure_score: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReport {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub measure: Canonical,
    pub subject: Option<Reference>,
    pub date: Option<DateTime>,
    pub reporter: Option<Reference>,
    pub period: Period,
    #[fhir_serde(rename = "improvementNotation")]
    pub improvement_notation: Option<CodeableConcept>,
    pub group: Option<Vec<MeasureReportGroup>>,
    #[fhir_serde(rename = "evaluatedResource")]
    pub evaluated_resource: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroupPopulation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub count: Option<Integer>,
    #[fhir_serde(rename = "subjectResults")]
    pub subject_results: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroupStratifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub stratum: Option<Vec<MeasureReportGroupStratifierStratum>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroupStratifierStratumComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: CodeableConcept,
}


/// Choice of types for the created\[x\] field in Media
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MediaCreated {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "createdDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "createdPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Media {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub modality: Option<CodeableConcept>,
    pub view: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub created: Option<MediaCreated>,
    pub issued: Option<Instant>,
    pub operator: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[fhir_serde(rename = "deviceName")]
    pub device_name: Option<String>,
    pub device: Option<Reference>,
    pub height: Option<PositiveInt>,
    pub width: Option<PositiveInt>,
    pub frames: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    pub content: Attachment,
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the item\[x\] field in MedicationIngredient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationIngredientItem {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationIngredient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub item: Option<MedicationIngredientItem>,
    #[fhir_serde(rename = "isActive")]
    pub is_active: Option<Boolean>,
    pub strength: Option<Ratio>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Medication {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<Code>,
    pub manufacturer: Option<Reference>,
    pub form: Option<CodeableConcept>,
    pub amount: Option<Ratio>,
    pub ingredient: Option<Vec<MedicationIngredient>>,
    pub batch: Option<MedicationBatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationBatch {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    #[fhir_serde(rename = "expirationDate")]
    pub expiration_date: Option<DateTime>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationAdministrationPerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

/// Choice of types for the medication\[x\] field in MedicationAdministration
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationAdministrationMedication {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the effective\[x\] field in MedicationAdministration
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationAdministrationEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationAdministration {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates: Option<Vec<Uri>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub medication: Option<MedicationAdministrationMedication>,
    pub subject: Reference,
    pub context: Option<Reference>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub effective: Option<MedicationAdministrationEffective>,
    pub performer: Option<Vec<MedicationAdministrationPerformer>>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub request: Option<Reference>,
    pub device: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage: Option<MedicationAdministrationDosage>,
    #[fhir_serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

/// Choice of types for the rate\[x\] field in MedicationAdministrationDosage
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationAdministrationDosageRate {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "rateRatio")]
    Ratio(Ratio),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "rateQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationAdministrationDosage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub text: Option<String>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub dose: Option<Quantity>,
    #[fhir_serde(flatten)]
    pub rate: Option<MedicationAdministrationDosageRate>,
}


/// Choice of types for the statusReason\[x\] field in MedicationDispense
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationDispenseStatusReason {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "statusReasonCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "statusReasonReference")]
    Reference(Reference),
}

/// Choice of types for the medication\[x\] field in MedicationDispense
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationDispenseMedication {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationDispense {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(flatten)]
    pub status_reason: Option<MedicationDispenseStatusReason>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub medication: Option<MedicationDispenseMedication>,
    pub subject: Option<Reference>,
    pub context: Option<Reference>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    pub performer: Option<Vec<MedicationDispensePerformer>>,
    pub location: Option<Reference>,
    #[fhir_serde(rename = "authorizingPrescription")]
    pub authorizing_prescription: Option<Vec<Reference>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "daysSupply")]
    pub days_supply: Option<Quantity>,
    #[fhir_serde(rename = "whenPrepared")]
    pub when_prepared: Option<DateTime>,
    #[fhir_serde(rename = "whenHandedOver")]
    pub when_handed_over: Option<DateTime>,
    pub destination: Option<Reference>,
    pub receiver: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "dosageInstruction")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    pub substitution: Option<MedicationDispenseSubstitution>,
    #[fhir_serde(rename = "detectedIssue")]
    pub detected_issue: Option<Vec<Reference>>,
    #[fhir_serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationDispensePerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationDispenseSubstitution {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "wasSubstituted")]
    pub was_substituted: Boolean,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub reason: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "responsibleParty")]
    pub responsible_party: Option<Vec<Reference>>,
}


/// Choice of types for the value\[x\] field in MedicationKnowledgeDrugCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationKnowledgeDrugCharacteristicValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeDrugCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<MedicationKnowledgeDrugCharacteristicValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeRegulatorySchedule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeRelatedMedicationKnowledge {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub reference: Option<Vec<Reference>>,
}

/// Choice of types for the indication\[x\] field in MedicationKnowledgeAdministrationGuidelines
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationKnowledgeAdministrationGuidelinesIndication {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "indicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "indicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeAdministrationGuidelines {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub dosage: Option<Vec<MedicationKnowledgeAdministrationGuidelinesDosage>>,
    #[fhir_serde(flatten)]
    pub indication: Option<MedicationKnowledgeAdministrationGuidelinesIndication>,
    #[fhir_serde(rename = "patientCharacteristics")]
    pub patient_characteristics: Option<Vec<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics>>,
}

/// Choice of types for the characteristic\[x\] field in MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationKnowledgeAdministrationGuidelinesPatientCharacteristicsCharacteristic {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "characteristicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "characteristicQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub characteristic: Option<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristicsCharacteristic>,
    pub value: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeCost {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub source: Option<String>,
    pub cost: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeMedicineClassification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub classification: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgePackaging {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeRegulatory {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "regulatoryAuthority")]
    pub regulatory_authority: Reference,
    pub substitution: Option<Vec<MedicationKnowledgeRegulatorySubstitution>>,
    pub schedule: Option<Vec<MedicationKnowledgeRegulatorySchedule>>,
    #[fhir_serde(rename = "maxDispense")]
    pub max_dispense: Option<MedicationKnowledgeRegulatoryMaxDispense>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeMonograph {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub source: Option<Reference>,
}

/// Choice of types for the item\[x\] field in MedicationKnowledgeIngredient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationKnowledgeIngredientItem {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeIngredient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub item: Option<MedicationKnowledgeIngredientItem>,
    #[fhir_serde(rename = "isActive")]
    pub is_active: Option<Boolean>,
    pub strength: Option<Ratio>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeRegulatoryMaxDispense {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Quantity,
    pub period: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeMonitoringProgram {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeRegulatorySubstitution {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub allowed: Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledge {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<Code>,
    pub manufacturer: Option<Reference>,
    #[fhir_serde(rename = "doseForm")]
    pub dose_form: Option<CodeableConcept>,
    pub amount: Option<Quantity>,
    pub synonym: Option<Vec<String>>,
    #[fhir_serde(rename = "relatedMedicationKnowledge")]
    pub related_medication_knowledge: Option<Vec<MedicationKnowledgeRelatedMedicationKnowledge>>,
    #[fhir_serde(rename = "associatedMedication")]
    pub associated_medication: Option<Vec<Reference>>,
    #[fhir_serde(rename = "productType")]
    pub product_type: Option<Vec<CodeableConcept>>,
    pub monograph: Option<Vec<MedicationKnowledgeMonograph>>,
    pub ingredient: Option<Vec<MedicationKnowledgeIngredient>>,
    #[fhir_serde(rename = "preparationInstruction")]
    pub preparation_instruction: Option<Markdown>,
    #[fhir_serde(rename = "intendedRoute")]
    pub intended_route: Option<Vec<CodeableConcept>>,
    pub cost: Option<Vec<MedicationKnowledgeCost>>,
    #[fhir_serde(rename = "monitoringProgram")]
    pub monitoring_program: Option<Vec<MedicationKnowledgeMonitoringProgram>>,
    #[fhir_serde(rename = "administrationGuidelines")]
    pub administration_guidelines: Option<Vec<MedicationKnowledgeAdministrationGuidelines>>,
    #[fhir_serde(rename = "medicineClassification")]
    pub medicine_classification: Option<Vec<MedicationKnowledgeMedicineClassification>>,
    pub packaging: Option<MedicationKnowledgePackaging>,
    #[fhir_serde(rename = "drugCharacteristic")]
    pub drug_characteristic: Option<Vec<MedicationKnowledgeDrugCharacteristic>>,
    pub contraindication: Option<Vec<Reference>>,
    pub regulatory: Option<Vec<MedicationKnowledgeRegulatory>>,
    pub kinetics: Option<Vec<MedicationKnowledgeKinetics>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeAdministrationGuidelinesDosage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub dosage: Option<Vec<Dosage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeKinetics {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "areaUnderCurve")]
    pub area_under_curve: Option<Vec<Quantity>>,
    #[fhir_serde(rename = "lethalDose50")]
    pub lethal_dose50: Option<Vec<Quantity>>,
    #[fhir_serde(rename = "halfLifePeriod")]
    pub half_life_period: Option<Duration>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationRequestDispenseRequest {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "initialFill")]
    pub initial_fill: Option<MedicationRequestDispenseRequestInitialFill>,
    #[fhir_serde(rename = "dispenseInterval")]
    pub dispense_interval: Option<Duration>,
    #[fhir_serde(rename = "validityPeriod")]
    pub validity_period: Option<Period>,
    #[fhir_serde(rename = "numberOfRepeatsAllowed")]
    pub number_of_repeats_allowed: Option<UnsignedInt>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "expectedSupplyDuration")]
    pub expected_supply_duration: Option<Duration>,
    pub performer: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationRequestDispenseRequestInitialFill {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    pub duration: Option<Duration>,
}

/// Choice of types for the reported\[x\] field in MedicationRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationRequestReported {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "reportedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "reportedReference")]
    Reference(Reference),
}

/// Choice of types for the medication\[x\] field in MedicationRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationRequestMedication {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationRequest {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub reported: Option<MedicationRequestReported>,
    #[fhir_serde(flatten)]
    pub medication: Option<MedicationRequestMedication>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub performer: Option<Reference>,
    #[fhir_serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub recorder: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    #[fhir_serde(rename = "courseOfTherapyType")]
    pub course_of_therapy_type: Option<CodeableConcept>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "dosageInstruction")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    #[fhir_serde(rename = "dispenseRequest")]
    pub dispense_request: Option<MedicationRequestDispenseRequest>,
    pub substitution: Option<MedicationRequestSubstitution>,
    #[fhir_serde(rename = "priorPrescription")]
    pub prior_prescription: Option<Reference>,
    #[fhir_serde(rename = "detectedIssue")]
    pub detected_issue: Option<Vec<Reference>>,
    #[fhir_serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

/// Choice of types for the allowed\[x\] field in MedicationRequestSubstitution
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationRequestSubstitutionAllowed {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "allowedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "allowedCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationRequestSubstitution {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub allowed: Option<MedicationRequestSubstitutionAllowed>,
    pub reason: Option<CodeableConcept>,
}


/// Choice of types for the medication\[x\] field in MedicationStatement
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationStatementMedication {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the effective\[x\] field in MedicationStatement
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicationStatementEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationStatement {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub medication: Option<MedicationStatementMedication>,
    pub subject: Reference,
    pub context: Option<Reference>,
    #[fhir_serde(flatten)]
    pub effective: Option<MedicationStatementEffective>,
    #[fhir_serde(rename = "dateAsserted")]
    pub date_asserted: Option<DateTime>,
    #[fhir_serde(rename = "informationSource")]
    pub information_source: Option<Reference>,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage: Option<Vec<Dosage>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinitionContact {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub contact: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinitionCrossReference {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub product: CodeableReference,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinitionNameCountryLanguage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    pub jurisdiction: Option<CodeableConcept>,
    pub language: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinitionOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableReference>,
    #[fhir_serde(rename = "effectiveDate")]
    pub effective_date: Option<Period>,
    pub organization: Option<Vec<Reference>>,
    #[fhir_serde(rename = "confidentialityIndicator")]
    pub confidentiality_indicator: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinitionName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "productName")]
    pub product_name: String,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "namePart")]
    pub name_part: Option<Vec<MedicinalProductDefinitionNameNamePart>>,
    #[fhir_serde(rename = "countryLanguage")]
    pub country_language: Option<Vec<MedicinalProductDefinitionNameCountryLanguage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinitionNameNamePart {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub part: String,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
}

/// Choice of types for the value\[x\] field in MedicinalProductDefinitionCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MedicinalProductDefinitionCharacteristicValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinitionCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<MedicinalProductDefinitionCharacteristicValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub domain: Option<CodeableConcept>,
    pub version: Option<String>,
    pub status: Option<CodeableConcept>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "combinedPharmaceuticalDoseForm")]
    pub combined_pharmaceutical_dose_form: Option<CodeableConcept>,
    pub route: Option<Vec<CodeableConcept>>,
    pub indication: Option<Markdown>,
    #[fhir_serde(rename = "legalStatusOfSupply")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[fhir_serde(rename = "additionalMonitoringIndicator")]
    pub additional_monitoring_indicator: Option<CodeableConcept>,
    #[fhir_serde(rename = "specialMeasures")]
    pub special_measures: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "pediatricUseIndicator")]
    pub pediatric_use_indicator: Option<CodeableConcept>,
    pub classification: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "marketingStatus")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    #[fhir_serde(rename = "packagedMedicinalProduct")]
    pub packaged_medicinal_product: Option<Vec<CodeableConcept>>,
    pub ingredient: Option<Vec<CodeableConcept>>,
    pub impurity: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "attachedDocument")]
    pub attached_document: Option<Vec<Reference>>,
    #[fhir_serde(rename = "masterFile")]
    pub master_file: Option<Vec<Reference>>,
    pub contact: Option<Vec<MedicinalProductDefinitionContact>>,
    #[fhir_serde(rename = "clinicalTrial")]
    pub clinical_trial: Option<Vec<Reference>>,
    pub code: Option<Vec<Coding>>,
    pub name: Option<Vec<MedicinalProductDefinitionName>>,
    #[fhir_serde(rename = "crossReference")]
    pub cross_reference: Option<Vec<MedicinalProductDefinitionCrossReference>>,
    pub operation: Option<Vec<MedicinalProductDefinitionOperation>>,
    pub characteristic: Option<Vec<MedicinalProductDefinitionCharacteristic>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MessageDefinitionAllowedResponse {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub message: Canonical,
    pub situation: Option<Markdown>,
}

/// Choice of types for the event\[x\] field in MessageDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MessageDefinitionEvent {
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MessageDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub replaces: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub base: Option<Canonical>,
    pub parent: Option<Vec<Canonical>>,
    #[fhir_serde(flatten)]
    pub event: Option<MessageDefinitionEvent>,
    pub category: Option<Code>,
    pub focus: Option<Vec<MessageDefinitionFocus>>,
    #[fhir_serde(rename = "responseRequired")]
    pub response_required: Option<Code>,
    #[fhir_serde(rename = "allowedResponse")]
    pub allowed_response: Option<Vec<MessageDefinitionAllowedResponse>>,
    pub graph: Option<Vec<Canonical>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MessageDefinitionFocus {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub profile: Option<Canonical>,
    pub min: UnsignedInt,
    pub max: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MessageHeaderDestination {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub target: Option<Reference>,
    pub endpoint: Url,
    pub receiver: Option<Reference>,
}

/// Choice of types for the event\[x\] field in MessageHeader
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum MessageHeaderEvent {
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MessageHeader {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub event: Option<MessageHeaderEvent>,
    pub destination: Option<Vec<MessageHeaderDestination>>,
    pub sender: Option<Reference>,
    pub enterer: Option<Reference>,
    pub author: Option<Reference>,
    pub source: MessageHeaderSource,
    pub responsible: Option<Reference>,
    pub reason: Option<CodeableConcept>,
    pub response: Option<MessageHeaderResponse>,
    pub focus: Option<Vec<Reference>>,
    pub definition: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MessageHeaderResponse {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Id,
    pub code: Code,
    pub details: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MessageHeaderSource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub software: Option<String>,
    pub version: Option<String>,
    pub contact: Option<ContactPoint>,
    pub endpoint: Url,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequenceStructureVariantOuter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequenceStructureVariantInner {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequenceReferenceSeq {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub chromosome: Option<CodeableConcept>,
    #[fhir_serde(rename = "genomeBuild")]
    pub genome_build: Option<String>,
    pub orientation: Option<Code>,
    #[fhir_serde(rename = "referenceSeqId")]
    pub reference_seq_id: Option<CodeableConcept>,
    #[fhir_serde(rename = "referenceSeqPointer")]
    pub reference_seq_pointer: Option<Reference>,
    #[fhir_serde(rename = "referenceSeqString")]
    pub reference_seq_string: Option<String>,
    pub strand: Option<Code>,
    #[fhir_serde(rename = "windowStart")]
    pub window_start: Option<Integer>,
    #[fhir_serde(rename = "windowEnd")]
    pub window_end: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequenceQualityRoc {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub score: Option<Vec<Integer>>,
    #[fhir_serde(rename = "numTP")]
    pub num_t_p: Option<Vec<Integer>>,
    #[fhir_serde(rename = "numFP")]
    pub num_f_p: Option<Vec<Integer>>,
    #[fhir_serde(rename = "numFN")]
    pub num_f_n: Option<Vec<Integer>>,
    pub precision: Option<Vec<Decimal>>,
    pub sensitivity: Option<Vec<Decimal>>,
    #[fhir_serde(rename = "fMeasure")]
    pub f_measure: Option<Vec<Decimal>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequenceVariant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
    #[fhir_serde(rename = "observedAllele")]
    pub observed_allele: Option<String>,
    #[fhir_serde(rename = "referenceAllele")]
    pub reference_allele: Option<String>,
    pub cigar: Option<String>,
    #[fhir_serde(rename = "variantPointer")]
    pub variant_pointer: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequenceStructureVariant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "variantType")]
    pub variant_type: Option<CodeableConcept>,
    pub exact: Option<Boolean>,
    pub length: Option<Integer>,
    pub outer: Option<MolecularSequenceStructureVariantOuter>,
    pub inner: Option<MolecularSequenceStructureVariantInner>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequenceQuality {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(rename = "standardSequence")]
    pub standard_sequence: Option<CodeableConcept>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
    pub score: Option<Quantity>,
    pub method: Option<CodeableConcept>,
    #[fhir_serde(rename = "truthTP")]
    pub truth_t_p: Option<Decimal>,
    #[fhir_serde(rename = "queryTP")]
    pub query_t_p: Option<Decimal>,
    #[fhir_serde(rename = "truthFN")]
    pub truth_f_n: Option<Decimal>,
    #[fhir_serde(rename = "queryFP")]
    pub query_f_p: Option<Decimal>,
    #[fhir_serde(rename = "gtFP")]
    pub gt_f_p: Option<Decimal>,
    pub precision: Option<Decimal>,
    pub recall: Option<Decimal>,
    #[fhir_serde(rename = "fScore")]
    pub f_score: Option<Decimal>,
    pub roc: Option<MolecularSequenceQualityRoc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequenceRepository {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub url: Option<Uri>,
    pub name: Option<String>,
    #[fhir_serde(rename = "datasetId")]
    pub dataset_id: Option<String>,
    #[fhir_serde(rename = "variantsetId")]
    pub variantset_id: Option<String>,
    #[fhir_serde(rename = "readsetId")]
    pub readset_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularSequence {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    #[fhir_serde(rename = "coordinateSystem")]
    pub coordinate_system: Integer,
    pub patient: Option<Reference>,
    pub specimen: Option<Reference>,
    pub device: Option<Reference>,
    pub performer: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "referenceSeq")]
    pub reference_seq: Option<MolecularSequenceReferenceSeq>,
    pub variant: Option<Vec<MolecularSequenceVariant>>,
    #[fhir_serde(rename = "observedSeq")]
    pub observed_seq: Option<String>,
    pub quality: Option<Vec<MolecularSequenceQuality>>,
    #[fhir_serde(rename = "readCoverage")]
    pub read_coverage: Option<Integer>,
    pub repository: Option<Vec<MolecularSequenceRepository>>,
    pub pointer: Option<Vec<Reference>>,
    #[fhir_serde(rename = "structureVariant")]
    pub structure_variant: Option<Vec<MolecularSequenceStructureVariant>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NamingSystemUniqueId {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub value: String,
    pub preferred: Option<Boolean>,
    pub comment: Option<String>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NamingSystem {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub status: Code,
    pub kind: Code,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub responsible: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub usage: Option<String>,
    #[fhir_serde(rename = "uniqueId")]
    pub unique_id: Option<Vec<NamingSystemUniqueId>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderOralDiet {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub schedule: Option<Vec<Timing>>,
    pub nutrient: Option<Vec<NutritionOrderOralDietNutrient>>,
    pub texture: Option<Vec<NutritionOrderOralDietTexture>>,
    #[fhir_serde(rename = "fluidConsistencyType")]
    pub fluid_consistency_type: Option<Vec<CodeableConcept>>,
    pub instruction: Option<String>,
}

/// Choice of types for the rate\[x\] field in NutritionOrderEnteralFormulaAdministration
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum NutritionOrderEnteralFormulaAdministrationRate {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "rateQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "rateRatio")]
    Ratio(Ratio),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderEnteralFormulaAdministration {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: Option<Timing>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(flatten)]
    pub rate: Option<NutritionOrderEnteralFormulaAdministrationRate>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderOralDietNutrient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub modifier: Option<CodeableConcept>,
    pub amount: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderEnteralFormula {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "baseFormulaType")]
    pub base_formula_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "baseFormulaProductName")]
    pub base_formula_product_name: Option<String>,
    #[fhir_serde(rename = "additiveType")]
    pub additive_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "additiveProductName")]
    pub additive_product_name: Option<String>,
    #[fhir_serde(rename = "caloricDensity")]
    pub caloric_density: Option<Quantity>,
    #[fhir_serde(rename = "routeofAdministration")]
    pub routeof_administration: Option<CodeableConcept>,
    pub administration: Option<Vec<NutritionOrderEnteralFormulaAdministration>>,
    #[fhir_serde(rename = "maxVolumeToDeliver")]
    pub max_volume_to_deliver: Option<Quantity>,
    #[fhir_serde(rename = "administrationInstruction")]
    pub administration_instruction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrder {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    pub instantiates: Option<Vec<Uri>>,
    pub status: Code,
    pub intent: Code,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "dateTime")]
    pub date_time: DateTime,
    pub orderer: Option<Reference>,
    #[fhir_serde(rename = "allergyIntolerance")]
    pub allergy_intolerance: Option<Vec<Reference>>,
    #[fhir_serde(rename = "foodPreferenceModifier")]
    pub food_preference_modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "excludeFoodModifier")]
    pub exclude_food_modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "oralDiet")]
    pub oral_diet: Option<NutritionOrderOralDiet>,
    pub supplement: Option<Vec<NutritionOrderSupplement>>,
    #[fhir_serde(rename = "enteralFormula")]
    pub enteral_formula: Option<NutritionOrderEnteralFormula>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderOralDietTexture {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub modifier: Option<CodeableConcept>,
    #[fhir_serde(rename = "foodType")]
    pub food_type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderSupplement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "productName")]
    pub product_name: Option<String>,
    pub schedule: Option<Vec<Timing>>,
    pub quantity: Option<Quantity>,
    pub instruction: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionProduct {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: Option<CodeableConcept>,
    pub manufacturer: Option<Vec<Reference>>,
    pub nutrient: Option<Vec<NutritionProductNutrient>>,
    pub ingredient: Option<Vec<NutritionProductIngredient>>,
    #[fhir_serde(rename = "knownAllergen")]
    pub known_allergen: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "productCharacteristic")]
    pub product_characteristic: Option<Vec<NutritionProductProductCharacteristic>>,
    pub instance: Option<NutritionProductInstance>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionProductIngredient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    pub amount: Option<Vec<Ratio>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionProductNutrient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: Option<CodeableReference>,
    pub amount: Option<Vec<Ratio>>,
}

/// Choice of types for the value\[x\] field in NutritionProductProductCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum NutritionProductProductCharacteristicValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionProductProductCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<NutritionProductProductCharacteristicValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionProductInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    pub expiry: Option<DateTime>,
    #[fhir_serde(rename = "useBy")]
    pub use_by: Option<DateTime>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ObservationReferenceRange {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub low: Option<Quantity>,
    pub high: Option<Quantity>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "appliesTo")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    pub age: Option<Range>,
    pub text: Option<String>,
}

/// Choice of types for the value\[x\] field in ObservationComponent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ObservationComponentValue {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ObservationComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<ObservationComponentValue>,
    #[fhir_serde(rename = "dataAbsentReason")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub interpretation: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "referenceRange")]
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
}

/// Choice of types for the effective\[x\] field in Observation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ObservationEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "effectiveTiming")]
    Timing(Timing),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "effectiveInstant")]
    Instant(Instant),
}

/// Choice of types for the value\[x\] field in Observation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ObservationValue {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Observation {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub focus: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub effective: Option<ObservationEffective>,
    pub issued: Option<Instant>,
    pub performer: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub value: Option<ObservationValue>,
    #[fhir_serde(rename = "dataAbsentReason")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub interpretation: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub specimen: Option<Reference>,
    pub device: Option<Reference>,
    #[fhir_serde(rename = "referenceRange")]
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
    #[fhir_serde(rename = "hasMember")]
    pub has_member: Option<Vec<Reference>>,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    pub component: Option<Vec<ObservationComponent>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ObservationDefinitionQualifiedInterval {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<Code>,
    pub range: Option<Range>,
    pub context: Option<CodeableConcept>,
    #[fhir_serde(rename = "appliesTo")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    pub gender: Option<Code>,
    pub age: Option<Range>,
    #[fhir_serde(rename = "gestationalAge")]
    pub gestational_age: Option<Range>,
    pub condition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ObservationDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "permittedDataType")]
    pub permitted_data_type: Option<Vec<Code>>,
    #[fhir_serde(rename = "multipleResultsAllowed")]
    pub multiple_results_allowed: Option<Boolean>,
    pub method: Option<CodeableConcept>,
    #[fhir_serde(rename = "preferredReportName")]
    pub preferred_report_name: Option<String>,
    #[fhir_serde(rename = "quantitativeDetails")]
    pub quantitative_details: Option<ObservationDefinitionQuantitativeDetails>,
    #[fhir_serde(rename = "qualifiedInterval")]
    pub qualified_interval: Option<Vec<ObservationDefinitionQualifiedInterval>>,
    #[fhir_serde(rename = "validCodedValueSet")]
    pub valid_coded_value_set: Option<Reference>,
    #[fhir_serde(rename = "normalCodedValueSet")]
    pub normal_coded_value_set: Option<Reference>,
    #[fhir_serde(rename = "abnormalCodedValueSet")]
    pub abnormal_coded_value_set: Option<Reference>,
    #[fhir_serde(rename = "criticalCodedValueSet")]
    pub critical_coded_value_set: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ObservationDefinitionQuantitativeDetails {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "customaryUnit")]
    pub customary_unit: Option<CodeableConcept>,
    pub unit: Option<CodeableConcept>,
    #[fhir_serde(rename = "conversionFactor")]
    pub conversion_factor: Option<Decimal>,
    #[fhir_serde(rename = "decimalPrecision")]
    pub decimal_precision: Option<Integer>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OperationDefinitionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub min: Integer,
    pub max: String,
    pub documentation: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    #[fhir_serde(rename = "targetProfile")]
    pub target_profile: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "searchType")]
    pub search_type: Option<Code>,
    pub binding: Option<OperationDefinitionParameterBinding>,
    #[fhir_serde(rename = "referencedFrom")]
    pub referenced_from: Option<Vec<OperationDefinitionParameterReferencedFrom>>,
    pub part: Option<Vec<OperationDefinitionParameter>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OperationDefinitionParameterBinding {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub strength: Code,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OperationDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub kind: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    #[fhir_serde(rename = "affectsState")]
    pub affects_state: Option<Boolean>,
    pub code: Code,
    pub comment: Option<Markdown>,
    pub base: Option<Canonical>,
    pub resource: Option<Vec<Code>>,
    pub system: Boolean,
    #[fhir_serde(rename = "type")]
    pub r#type: Boolean,
    pub instance: Boolean,
    #[fhir_serde(rename = "inputProfile")]
    pub input_profile: Option<Canonical>,
    #[fhir_serde(rename = "outputProfile")]
    pub output_profile: Option<Canonical>,
    pub parameter: Option<Vec<OperationDefinitionParameter>>,
    pub overload: Option<Vec<OperationDefinitionOverload>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OperationDefinitionParameterReferencedFrom {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: String,
    #[fhir_serde(rename = "sourceId")]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OperationDefinitionOverload {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "parameterName")]
    pub parameter_name: Option<Vec<String>>,
    pub comment: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OperationOutcome {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub issue: Option<Vec<OperationOutcomeIssue>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OperationOutcomeIssue {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub severity: Code,
    pub code: Code,
    pub details: Option<CodeableConcept>,
    pub diagnostics: Option<String>,
    pub location: Option<Vec<String>>,
    pub expression: Option<Vec<String>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Organization {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Vec<Address>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Reference>,
    pub contact: Option<Vec<OrganizationContact>>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OrganizationContact {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub purpose: Option<CodeableConcept>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OrganizationAffiliation {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub period: Option<Period>,
    pub organization: Option<Reference>,
    #[fhir_serde(rename = "participatingOrganization")]
    pub participating_organization: Option<Reference>,
    pub network: Option<Vec<Reference>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    #[fhir_serde(rename = "healthcareService")]
    pub healthcare_service: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PackagedProductDefinitionLegalStatusOfSupply {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub jurisdiction: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PackagedProductDefinitionPackage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Integer>,
    pub material: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "alternateMaterial")]
    pub alternate_material: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "shelfLifeStorage")]
    pub shelf_life_storage: Option<Vec<PackagedProductDefinitionPackageShelfLifeStorage>>,
    pub manufacturer: Option<Vec<Reference>>,
    pub property: Option<Vec<PackagedProductDefinitionPackageProperty>>,
    #[fhir_serde(rename = "containedItem")]
    pub contained_item: Option<Vec<PackagedProductDefinitionPackageContainedItem>>,
    pub package: Option<Vec<PackagedProductDefinitionPackage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PackagedProductDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub name: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "packageFor")]
    pub package_for: Option<Vec<Reference>>,
    pub status: Option<CodeableConcept>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    #[fhir_serde(rename = "containedItemQuantity")]
    pub contained_item_quantity: Option<Vec<Quantity>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "legalStatusOfSupply")]
    pub legal_status_of_supply: Option<Vec<PackagedProductDefinitionLegalStatusOfSupply>>,
    #[fhir_serde(rename = "marketingStatus")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    pub characteristic: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "copackagedIndicator")]
    pub copackaged_indicator: Option<Boolean>,
    pub manufacturer: Option<Vec<Reference>>,
    pub package: Option<PackagedProductDefinitionPackage>,
}

/// Choice of types for the value\[x\] field in PackagedProductDefinitionPackageProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PackagedProductDefinitionPackagePropertyValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PackagedProductDefinitionPackageProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<PackagedProductDefinitionPackagePropertyValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PackagedProductDefinitionPackageContainedItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    pub amount: Option<Quantity>,
}

/// Choice of types for the period\[x\] field in PackagedProductDefinitionPackageShelfLifeStorage
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PackagedProductDefinitionPackageShelfLifeStoragePeriod {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "periodDuration")]
    Duration(Duration),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "periodString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PackagedProductDefinitionPackageShelfLifeStorage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub period: Option<PackagedProductDefinitionPackageShelfLifeStoragePeriod>,
    #[fhir_serde(rename = "specialPrecautionsForStorage")]
    pub special_precautions_for_storage: Option<Vec<CodeableConcept>>,
}


/// Choice of types for the value\[x\] field in ParametersParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ParametersParameterValue {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ParametersParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(flatten)]
    pub value: Option<ParametersParameterValue>,
    pub resource: Option<Resource>,
    pub part: Option<Vec<ParametersParameter>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Parameters {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub parameter: Option<Vec<ParametersParameter>>,
}


/// Choice of types for the deceased\[x\] field in Patient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PatientDeceased {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "deceasedDateTime")]
    DateTime(DateTime),
}

/// Choice of types for the multipleBirth\[x\] field in Patient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PatientMultipleBirth {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "multipleBirthBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "multipleBirthInteger")]
    Integer(Integer),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Patient {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    #[fhir_serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    #[fhir_serde(flatten)]
    pub deceased: Option<PatientDeceased>,
    pub address: Option<Vec<Address>>,
    #[fhir_serde(rename = "maritalStatus")]
    pub marital_status: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub multiple_birth: Option<PatientMultipleBirth>,
    pub photo: Option<Vec<Attachment>>,
    pub contact: Option<Vec<PatientContact>>,
    pub communication: Option<Vec<PatientCommunication>>,
    #[fhir_serde(rename = "generalPractitioner")]
    pub general_practitioner: Option<Vec<Reference>>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub link: Option<Vec<PatientLink>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PatientContact {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationship: Option<Vec<CodeableConcept>>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
    pub gender: Option<Code>,
    pub organization: Option<Reference>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PatientCommunication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PatientLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub other: Reference,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PaymentNotice {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub request: Option<Reference>,
    pub response: Option<Reference>,
    pub created: DateTime,
    pub provider: Option<Reference>,
    pub payment: Reference,
    #[fhir_serde(rename = "paymentDate")]
    pub payment_date: Option<Date>,
    pub payee: Option<Reference>,
    pub recipient: Reference,
    pub amount: Money,
    #[fhir_serde(rename = "paymentStatus")]
    pub payment_status: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PaymentReconciliationProcessNote {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PaymentReconciliation {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub period: Option<Period>,
    pub created: DateTime,
    #[fhir_serde(rename = "paymentIssuer")]
    pub payment_issuer: Option<Reference>,
    pub request: Option<Reference>,
    pub requestor: Option<Reference>,
    pub outcome: Option<Code>,
    pub disposition: Option<String>,
    #[fhir_serde(rename = "paymentDate")]
    pub payment_date: Date,
    #[fhir_serde(rename = "paymentAmount")]
    pub payment_amount: Money,
    #[fhir_serde(rename = "paymentIdentifier")]
    pub payment_identifier: Option<Identifier>,
    pub detail: Option<Vec<PaymentReconciliationDetail>>,
    #[fhir_serde(rename = "formCode")]
    pub form_code: Option<CodeableConcept>,
    #[fhir_serde(rename = "processNote")]
    pub process_note: Option<Vec<PaymentReconciliationProcessNote>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PaymentReconciliationDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub predecessor: Option<Identifier>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub request: Option<Reference>,
    pub submitter: Option<Reference>,
    pub response: Option<Reference>,
    pub date: Option<Date>,
    pub responsible: Option<Reference>,
    pub payee: Option<Reference>,
    pub amount: Option<Money>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Person {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    #[fhir_serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    pub address: Option<Vec<Address>>,
    pub photo: Option<Attachment>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub active: Option<Boolean>,
    pub link: Option<Vec<PersonLink>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PersonLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Reference,
    pub assurance: Option<Code>,
}


/// Choice of types for the subject\[x\] field in PlanDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PlanDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "subjectCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub subject: Option<PlanDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub goal: Option<Vec<PlanDefinitionGoal>>,
    pub action: Option<Vec<PlanDefinitionAction>>,
}

/// Choice of types for the detail\[x\] field in PlanDefinitionGoalTarget
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PlanDefinitionGoalTargetDetail {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "detailQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "detailRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "detailCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionGoalTarget {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub measure: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub detail: Option<PlanDefinitionGoalTargetDetail>,
    pub due: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionActionDynamicValue {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub expression: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionActionCondition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    pub expression: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionGoal {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub priority: Option<CodeableConcept>,
    pub start: Option<CodeableConcept>,
    pub addresses: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    pub target: Option<Vec<PlanDefinitionGoalTarget>>,
}

/// Choice of types for the subject\[x\] field in PlanDefinitionAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PlanDefinitionActionSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "subjectCanonical")]
    Canonical(Canonical),
}

/// Choice of types for the timing\[x\] field in PlanDefinitionAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PlanDefinitionActionTiming {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "timingDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "timingTiming")]
    Timing(Timing),
}

/// Choice of types for the definition\[x\] field in PlanDefinitionAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PlanDefinitionActionDefinition {
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "definitionUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub prefix: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    #[fhir_serde(rename = "textEquivalent")]
    pub text_equivalent: Option<String>,
    pub priority: Option<Code>,
    pub code: Option<Vec<CodeableConcept>>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[fhir_serde(rename = "goalId")]
    pub goal_id: Option<Vec<Id>>,
    #[fhir_serde(flatten)]
    pub subject: Option<PlanDefinitionActionSubject>,
    pub trigger: Option<Vec<TriggerDefinition>>,
    pub condition: Option<Vec<PlanDefinitionActionCondition>>,
    pub input: Option<Vec<DataRequirement>>,
    pub output: Option<Vec<DataRequirement>>,
    #[fhir_serde(rename = "relatedAction")]
    pub related_action: Option<Vec<PlanDefinitionActionRelatedAction>>,
    #[fhir_serde(flatten)]
    pub timing: Option<PlanDefinitionActionTiming>,
    pub participant: Option<Vec<PlanDefinitionActionParticipant>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "groupingBehavior")]
    pub grouping_behavior: Option<Code>,
    #[fhir_serde(rename = "selectionBehavior")]
    pub selection_behavior: Option<Code>,
    #[fhir_serde(rename = "requiredBehavior")]
    pub required_behavior: Option<Code>,
    #[fhir_serde(rename = "precheckBehavior")]
    pub precheck_behavior: Option<Code>,
    #[fhir_serde(rename = "cardinalityBehavior")]
    pub cardinality_behavior: Option<Code>,
    #[fhir_serde(flatten)]
    pub definition: Option<PlanDefinitionActionDefinition>,
    pub transform: Option<Canonical>,
    #[fhir_serde(rename = "dynamicValue")]
    pub dynamic_value: Option<Vec<PlanDefinitionActionDynamicValue>>,
    pub action: Option<Vec<PlanDefinitionAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionActionParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub role: Option<CodeableConcept>,
}

/// Choice of types for the offset\[x\] field in PlanDefinitionActionRelatedAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PlanDefinitionActionRelatedActionOffset {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionActionRelatedAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[fhir_serde(flatten)]
    pub offset: Option<PlanDefinitionActionRelatedActionOffset>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Practitioner {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Vec<Address>>,
    pub gender: Option<Code>,
    #[fhir_serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    pub photo: Option<Vec<Attachment>>,
    pub qualification: Option<Vec<PractitionerQualification>>,
    pub communication: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PractitionerQualification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    pub period: Option<Period>,
    pub issuer: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PractitionerRoleAvailableTime {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "daysOfWeek")]
    pub days_of_week: Option<Vec<Code>>,
    #[fhir_serde(rename = "allDay")]
    pub all_day: Option<Boolean>,
    #[fhir_serde(rename = "availableStartTime")]
    pub available_start_time: Option<Time>,
    #[fhir_serde(rename = "availableEndTime")]
    pub available_end_time: Option<Time>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PractitionerRole {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub period: Option<Period>,
    pub practitioner: Option<Reference>,
    pub organization: Option<Reference>,
    pub code: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    #[fhir_serde(rename = "healthcareService")]
    pub healthcare_service: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    #[fhir_serde(rename = "availableTime")]
    pub available_time: Option<Vec<PractitionerRoleAvailableTime>>,
    #[fhir_serde(rename = "notAvailable")]
    pub not_available: Option<Vec<PractitionerRoleNotAvailable>>,
    #[fhir_serde(rename = "availabilityExceptions")]
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PractitionerRoleNotAvailable {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub during: Option<Period>,
}


/// Choice of types for the performed\[x\] field in Procedure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ProcedurePerformed {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "performedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "performedPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "performedString")]
    String(String),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "performedAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "performedRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Procedure {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub performed: Option<ProcedurePerformed>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    pub performer: Option<Vec<ProcedurePerformer>>,
    pub location: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    pub outcome: Option<CodeableConcept>,
    pub report: Option<Vec<Reference>>,
    pub complication: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "complicationDetail")]
    pub complication_detail: Option<Vec<Reference>>,
    #[fhir_serde(rename = "followUp")]
    pub follow_up: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "focalDevice")]
    pub focal_device: Option<Vec<ProcedureFocalDevice>>,
    #[fhir_serde(rename = "usedReference")]
    pub used_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "usedCode")]
    pub used_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ProcedurePerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[fhir_serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ProcedureFocalDevice {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<CodeableConcept>,
    pub manipulated: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ProvenanceAgent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Reference,
    #[fhir_serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
}

/// Choice of types for the occurred\[x\] field in Provenance
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ProvenanceOccurred {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurredPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurredDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Provenance {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub occurred: Option<ProvenanceOccurred>,
    pub recorded: Instant,
    pub policy: Option<Vec<Uri>>,
    pub location: Option<Reference>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub activity: Option<CodeableConcept>,
    pub agent: Option<Vec<ProvenanceAgent>>,
    pub entity: Option<Vec<ProvenanceEntity>>,
    pub signature: Option<Vec<Signature>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ProvenanceEntity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Code,
    pub what: Reference,
    pub agent: Option<Vec<ProvenanceAgent>>,
}


/// Choice of types for the value\[x\] field in QuestionnaireItemInitial
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum QuestionnaireItemInitialValue {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct QuestionnaireItemInitial {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<QuestionnaireItemInitialValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Questionnaire {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(rename = "subjectType")]
    pub subject_type: Option<Vec<Code>>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub code: Option<Vec<Coding>>,
    pub item: Option<Vec<QuestionnaireItem>>,
}

/// Choice of types for the answer\[x\] field in QuestionnaireItemEnableWhen
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum QuestionnaireItemEnableWhenAnswer {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "answerBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "answerDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "answerInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "answerDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "answerDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "answerTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "answerString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "answerCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "answerQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "answerReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct QuestionnaireItemEnableWhen {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub question: String,
    pub operator: Code,
    #[fhir_serde(flatten)]
    pub answer: Option<QuestionnaireItemEnableWhenAnswer>,
}

/// Choice of types for the value\[x\] field in QuestionnaireItemAnswerOption
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum QuestionnaireItemAnswerOptionValue {
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct QuestionnaireItemAnswerOption {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<QuestionnaireItemAnswerOptionValue>,
    #[fhir_serde(rename = "initialSelected")]
    pub initial_selected: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct QuestionnaireItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: String,
    pub definition: Option<Uri>,
    pub code: Option<Vec<Coding>>,
    pub prefix: Option<String>,
    pub text: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(rename = "enableWhen")]
    pub enable_when: Option<Vec<QuestionnaireItemEnableWhen>>,
    #[fhir_serde(rename = "enableBehavior")]
    pub enable_behavior: Option<Code>,
    pub required: Option<Boolean>,
    pub repeats: Option<Boolean>,
    #[fhir_serde(rename = "readOnly")]
    pub read_only: Option<Boolean>,
    #[fhir_serde(rename = "maxLength")]
    pub max_length: Option<Integer>,
    #[fhir_serde(rename = "answerValueSet")]
    pub answer_value_set: Option<Canonical>,
    #[fhir_serde(rename = "answerOption")]
    pub answer_option: Option<Vec<QuestionnaireItemAnswerOption>>,
    pub initial: Option<Vec<QuestionnaireItemInitial>>,
    pub item: Option<Vec<QuestionnaireItem>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct QuestionnaireResponse {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub questionnaire: Option<Canonical>,
    pub status: Code,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub authored: Option<DateTime>,
    pub author: Option<Reference>,
    pub source: Option<Reference>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct QuestionnaireResponseItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: String,
    pub definition: Option<Uri>,
    pub text: Option<String>,
    pub answer: Option<Vec<QuestionnaireResponseItemAnswer>>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

/// Choice of types for the value\[x\] field in QuestionnaireResponseItemAnswer
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum QuestionnaireResponseItemAnswerValue {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct QuestionnaireResponseItemAnswer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<QuestionnaireResponseItemAnswerValue>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RegulatedAuthorization {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub subject: Option<Vec<Reference>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub region: Option<Vec<CodeableConcept>>,
    pub status: Option<CodeableConcept>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    #[fhir_serde(rename = "validityPeriod")]
    pub validity_period: Option<Period>,
    pub indication: Option<CodeableReference>,
    #[fhir_serde(rename = "intendedUse")]
    pub intended_use: Option<CodeableConcept>,
    pub basis: Option<Vec<CodeableConcept>>,
    pub holder: Option<Reference>,
    pub regulator: Option<Reference>,
    pub case: Option<RegulatedAuthorizationCase>,
}

/// Choice of types for the date\[x\] field in RegulatedAuthorizationCase
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum RegulatedAuthorizationCaseDate {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "datePeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "dateDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RegulatedAuthorizationCase {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub date: Option<RegulatedAuthorizationCaseDate>,
    pub application: Option<Vec<RegulatedAuthorizationCase>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RelatedPersonCommunication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RelatedPerson {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub patient: Reference,
    pub relationship: Option<Vec<CodeableConcept>>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    #[fhir_serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    pub address: Option<Vec<Address>>,
    pub photo: Option<Vec<Attachment>>,
    pub period: Option<Period>,
    pub communication: Option<Vec<RelatedPersonCommunication>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequestGroupActionCondition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    pub expression: Option<Expression>,
}

/// Choice of types for the timing\[x\] field in RequestGroupAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum RequestGroupActionTiming {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "timingDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "timingTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequestGroupAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub prefix: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    #[fhir_serde(rename = "textEquivalent")]
    pub text_equivalent: Option<String>,
    pub priority: Option<Code>,
    pub code: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    pub condition: Option<Vec<RequestGroupActionCondition>>,
    #[fhir_serde(rename = "relatedAction")]
    pub related_action: Option<Vec<RequestGroupActionRelatedAction>>,
    #[fhir_serde(flatten)]
    pub timing: Option<RequestGroupActionTiming>,
    pub participant: Option<Vec<Reference>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "groupingBehavior")]
    pub grouping_behavior: Option<Code>,
    #[fhir_serde(rename = "selectionBehavior")]
    pub selection_behavior: Option<Code>,
    #[fhir_serde(rename = "requiredBehavior")]
    pub required_behavior: Option<Code>,
    #[fhir_serde(rename = "precheckBehavior")]
    pub precheck_behavior: Option<Code>,
    #[fhir_serde(rename = "cardinalityBehavior")]
    pub cardinality_behavior: Option<Code>,
    pub resource: Option<Reference>,
    pub action: Option<Vec<RequestGroupAction>>,
}

/// Choice of types for the offset\[x\] field in RequestGroupActionRelatedAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum RequestGroupActionRelatedActionOffset {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequestGroupActionRelatedAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[fhir_serde(flatten)]
    pub offset: Option<RequestGroupActionRelatedActionOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequestGroup {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    pub priority: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub author: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub action: Option<Vec<RequestGroupAction>>,
}


/// Choice of types for the subject\[x\] field in ResearchDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ResearchDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "shortTitle")]
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub subject: Option<ResearchDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub comment: Option<Vec<String>>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub population: Reference,
    pub exposure: Option<Reference>,
    #[fhir_serde(rename = "exposureAlternative")]
    pub exposure_alternative: Option<Reference>,
    pub outcome: Option<Reference>,
}


/// Choice of types for the subject\[x\] field in ResearchElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ResearchElementDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchElementDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "shortTitle")]
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub subject: Option<ResearchElementDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub comment: Option<Vec<String>>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(rename = "variableType")]
    pub variable_type: Option<Code>,
    pub characteristic: Option<Vec<ResearchElementDefinitionCharacteristic>>,
}

/// Choice of types for the definition\[x\] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ResearchElementDefinitionCharacteristicDefinition {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "definitionCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "definitionExpression")]
    Expression(Expression),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "definitionDataRequirement")]
    DataRequirement(DataRequirement),
}

/// Choice of types for the studyEffective\[x\] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ResearchElementDefinitionCharacteristicStudyEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "studyEffectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "studyEffectivePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "studyEffectiveDuration")]
    Duration(Duration),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "studyEffectiveTiming")]
    Timing(Timing),
}

/// Choice of types for the participantEffective\[x\] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ResearchElementDefinitionCharacteristicParticipantEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "participantEffectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "participantEffectivePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "participantEffectiveDuration")]
    Duration(Duration),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "participantEffectiveTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchElementDefinitionCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub definition: Option<ResearchElementDefinitionCharacteristicDefinition>,
    #[fhir_serde(rename = "usageContext")]
    pub usage_context: Option<Vec<UsageContext>>,
    pub exclude: Option<Boolean>,
    #[fhir_serde(rename = "unitOfMeasure")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[fhir_serde(rename = "studyEffectiveDescription")]
    pub study_effective_description: Option<String>,
    #[fhir_serde(flatten)]
    pub study_effective: Option<ResearchElementDefinitionCharacteristicStudyEffective>,
    #[fhir_serde(rename = "studyEffectiveTimeFromStart")]
    pub study_effective_time_from_start: Option<Duration>,
    #[fhir_serde(rename = "studyEffectiveGroupMeasure")]
    pub study_effective_group_measure: Option<Code>,
    #[fhir_serde(rename = "participantEffectiveDescription")]
    pub participant_effective_description: Option<String>,
    #[fhir_serde(flatten)]
    pub participant_effective: Option<ResearchElementDefinitionCharacteristicParticipantEffective>,
    #[fhir_serde(rename = "participantEffectiveTimeFromStart")]
    pub participant_effective_time_from_start: Option<Duration>,
    #[fhir_serde(rename = "participantEffectiveGroupMeasure")]
    pub participant_effective_group_measure: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudy {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub title: Option<String>,
    pub protocol: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "primaryPurposeType")]
    pub primary_purpose_type: Option<CodeableConcept>,
    pub phase: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub focus: Option<Vec<CodeableConcept>>,
    pub condition: Option<Vec<CodeableConcept>>,
    pub contact: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub keyword: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
    pub enrollment: Option<Vec<Reference>>,
    pub period: Option<Period>,
    pub sponsor: Option<Reference>,
    #[fhir_serde(rename = "principalInvestigator")]
    pub principal_investigator: Option<Reference>,
    pub site: Option<Vec<Reference>>,
    #[fhir_serde(rename = "reasonStopped")]
    pub reason_stopped: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
    pub arm: Option<Vec<ResearchStudyArm>>,
    pub objective: Option<Vec<ResearchStudyObjective>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyArm {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyObjective {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchSubject {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub period: Option<Period>,
    pub study: Reference,
    pub individual: Reference,
    #[fhir_serde(rename = "assignedArm")]
    pub assigned_arm: Option<String>,
    #[fhir_serde(rename = "actualArm")]
    pub actual_arm: Option<String>,
    pub consent: Option<Reference>,
}


/// Choice of types for the occurrence\[x\] field in RiskAssessment
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum RiskAssessmentOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RiskAssessment {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Reference>,
    pub parent: Option<Reference>,
    pub status: Code,
    pub method: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<RiskAssessmentOccurrence>,
    pub condition: Option<Reference>,
    pub performer: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub basis: Option<Vec<Reference>>,
    pub prediction: Option<Vec<RiskAssessmentPrediction>>,
    pub mitigation: Option<String>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the probability\[x\] field in RiskAssessmentPrediction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum RiskAssessmentPredictionProbability {
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "probabilityDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "probabilityRange")]
    Range(Range),
}

/// Choice of types for the when\[x\] field in RiskAssessmentPrediction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum RiskAssessmentPredictionWhen {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "whenPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "whenRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RiskAssessmentPrediction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub outcome: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub probability: Option<RiskAssessmentPredictionProbability>,
    #[fhir_serde(rename = "qualitativeRisk")]
    pub qualitative_risk: Option<CodeableConcept>,
    #[fhir_serde(rename = "relativeRisk")]
    pub relative_risk: Option<Decimal>,
    #[fhir_serde(flatten)]
    pub when: Option<RiskAssessmentPredictionWhen>,
    pub rationale: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Schedule {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    #[fhir_serde(rename = "serviceCategory")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub actor: Option<Vec<Reference>>,
    #[fhir_serde(rename = "planningHorizon")]
    pub planning_horizon: Option<Period>,
    pub comment: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SearchParameterComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub definition: Canonical,
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SearchParameter {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub version: Option<String>,
    pub name: String,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Canonical>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Markdown,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub code: Code,
    pub base: Option<Vec<Code>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub expression: Option<String>,
    pub xpath: Option<String>,
    #[fhir_serde(rename = "xpathUsage")]
    pub xpath_usage: Option<Code>,
    pub target: Option<Vec<Code>>,
    #[fhir_serde(rename = "multipleOr")]
    pub multiple_or: Option<Boolean>,
    #[fhir_serde(rename = "multipleAnd")]
    pub multiple_and: Option<Boolean>,
    pub comparator: Option<Vec<Code>>,
    pub modifier: Option<Vec<Code>>,
    pub chain: Option<Vec<String>>,
    pub component: Option<Vec<SearchParameterComponent>>,
}


/// Choice of types for the quantity\[x\] field in ServiceRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ServiceRequestQuantity {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "quantityQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "quantityRatio")]
    Ratio(Ratio),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "quantityRange")]
    Range(Range),
}

/// Choice of types for the occurrence\[x\] field in ServiceRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ServiceRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

/// Choice of types for the asNeeded\[x\] field in ServiceRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ServiceRequestAsNeeded {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ServiceRequest {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    pub requisition: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(rename = "orderDetail")]
    pub order_detail: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub quantity: Option<ServiceRequestQuantity>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<ServiceRequestOccurrence>,
    #[fhir_serde(flatten)]
    pub as_needed: Option<ServiceRequestAsNeeded>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    #[fhir_serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub performer: Option<Vec<Reference>>,
    #[fhir_serde(rename = "locationCode")]
    pub location_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "locationReference")]
    pub location_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub insurance: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub specimen: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "patientInstruction")]
    pub patient_instruction: Option<String>,
    #[fhir_serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Slot {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "serviceCategory")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "appointmentType")]
    pub appointment_type: Option<CodeableConcept>,
    pub schedule: Reference,
    pub status: Code,
    pub start: Instant,
    pub end: Instant,
    pub overbooked: Option<Boolean>,
    pub comment: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Specimen {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "accessionIdentifier")]
    pub accession_identifier: Option<Identifier>,
    pub status: Option<Code>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    #[fhir_serde(rename = "receivedTime")]
    pub received_time: Option<DateTime>,
    pub parent: Option<Vec<Reference>>,
    pub request: Option<Vec<Reference>>,
    pub collection: Option<SpecimenCollection>,
    pub processing: Option<Vec<SpecimenProcessing>>,
    pub container: Option<Vec<SpecimenContainer>>,
    pub condition: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the time\[x\] field in SpecimenProcessing
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SpecimenProcessingTime {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenProcessing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub procedure: Option<CodeableConcept>,
    pub additive: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub time: Option<SpecimenProcessingTime>,
}

/// Choice of types for the collected\[x\] field in SpecimenCollection
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SpecimenCollectionCollected {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "collectedPeriod")]
    Period(Period),
}

/// Choice of types for the fastingStatus\[x\] field in SpecimenCollection
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SpecimenCollectionFastingStatus {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "fastingStatusCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "fastingStatusDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenCollection {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub collector: Option<Reference>,
    #[fhir_serde(flatten)]
    pub collected: Option<SpecimenCollectionCollected>,
    pub duration: Option<Duration>,
    pub quantity: Option<Quantity>,
    pub method: Option<CodeableConcept>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub fasting_status: Option<SpecimenCollectionFastingStatus>,
}

/// Choice of types for the additive\[x\] field in SpecimenContainer
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SpecimenContainerAdditive {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenContainer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub description: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub capacity: Option<Quantity>,
    #[fhir_serde(rename = "specimenQuantity")]
    pub specimen_quantity: Option<Quantity>,
    #[fhir_serde(flatten)]
    pub additive: Option<SpecimenContainerAdditive>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenDefinitionTypeTestedHandling {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "temperatureQualifier")]
    pub temperature_qualifier: Option<CodeableConcept>,
    #[fhir_serde(rename = "temperatureRange")]
    pub temperature_range: Option<Range>,
    #[fhir_serde(rename = "maxDuration")]
    pub max_duration: Option<Duration>,
    pub instruction: Option<String>,
}

/// Choice of types for the minimumVolume\[x\] field in SpecimenDefinitionTypeTestedContainer
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SpecimenDefinitionTypeTestedContainerMinimumVolume {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "minimumVolumeQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "minimumVolumeString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenDefinitionTypeTestedContainer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub material: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub cap: Option<CodeableConcept>,
    pub description: Option<String>,
    pub capacity: Option<Quantity>,
    #[fhir_serde(flatten)]
    pub minimum_volume: Option<SpecimenDefinitionTypeTestedContainerMinimumVolume>,
    pub additive: Option<Vec<SpecimenDefinitionTypeTestedContainerAdditive>>,
    pub preparation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenDefinitionTypeTested {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "isDerived")]
    pub is_derived: Option<Boolean>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub preference: Code,
    pub container: Option<SpecimenDefinitionTypeTestedContainer>,
    pub requirement: Option<String>,
    #[fhir_serde(rename = "retentionTime")]
    pub retention_time: Option<Duration>,
    #[fhir_serde(rename = "rejectionCriterion")]
    pub rejection_criterion: Option<Vec<CodeableConcept>>,
    pub handling: Option<Vec<SpecimenDefinitionTypeTestedHandling>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[fhir_serde(rename = "typeCollected")]
    pub type_collected: Option<CodeableConcept>,
    #[fhir_serde(rename = "patientPreparation")]
    pub patient_preparation: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "timeAspect")]
    pub time_aspect: Option<String>,
    pub collection: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "typeTested")]
    pub type_tested: Option<Vec<SpecimenDefinitionTypeTested>>,
}

/// Choice of types for the additive\[x\] field in SpecimenDefinitionTypeTestedContainerAdditive
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SpecimenDefinitionTypeTestedContainerAdditiveAdditive {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenDefinitionTypeTestedContainerAdditive {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub additive: Option<SpecimenDefinitionTypeTestedContainerAdditiveAdditive>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureDefinitionMapping {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identity: Id,
    pub uri: Option<Uri>,
    pub name: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureDefinitionSnapshot {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureDefinitionContext {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub keyword: Option<Vec<Coding>>,
    #[fhir_serde(rename = "fhirVersion")]
    pub fhir_version: Option<Code>,
    pub mapping: Option<Vec<StructureDefinitionMapping>>,
    pub kind: Code,
    #[fhir_serde(rename = "abstract")]
    pub r#abstract: Boolean,
    pub context: Option<Vec<StructureDefinitionContext>>,
    #[fhir_serde(rename = "contextInvariant")]
    pub context_invariant: Option<Vec<String>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Uri,
    #[fhir_serde(rename = "baseDefinition")]
    pub base_definition: Option<Canonical>,
    pub derivation: Option<Code>,
    pub snapshot: Option<StructureDefinitionSnapshot>,
    pub differential: Option<StructureDefinitionDifferential>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureDefinitionDifferential {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupInput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<String>,
    pub mode: Code,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMap {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub structure: Option<Vec<StructureMapStructure>>,
    pub import: Option<Vec<Canonical>>,
    pub group: Option<Vec<StructureMapGroup>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRuleDependent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub variable: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapStructure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Canonical,
    pub mode: Code,
    pub alias: Option<String>,
    pub documentation: Option<String>,
}

/// Choice of types for the defaultValue\[x\] field in StructureMapGroupRuleSource
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum StructureMapGroupRuleSourceDefaultValue {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "defaultValueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "defaultValueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "defaultValueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "defaultValueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "defaultValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "defaultValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "defaultValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "defaultValueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "defaultValueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "defaultValueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "defaultValueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "defaultValueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "defaultValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "defaultValueString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "defaultValueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "defaultValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "defaultValueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "defaultValueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "defaultValueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "defaultValueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "defaultValueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "defaultValueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "defaultValueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "defaultValueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "defaultValueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "defaultValueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "defaultValueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "defaultValueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "defaultValueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "defaultValueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "defaultValueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "defaultValueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "defaultValuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "defaultValueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "defaultValueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "defaultValueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "defaultValueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "defaultValueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "defaultValueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "defaultValueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "defaultValueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "defaultValueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "defaultValueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "defaultValueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "defaultValueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "defaultValueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "defaultValueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "defaultValueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "defaultValueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "defaultValueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRuleSource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Id,
    pub min: Option<Integer>,
    pub max: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<String>,
    #[fhir_serde(flatten)]
    pub default_value: Option<StructureMapGroupRuleSourceDefaultValue>,
    pub element: Option<String>,
    #[fhir_serde(rename = "listMode")]
    pub list_mode: Option<Code>,
    pub variable: Option<Id>,
    pub condition: Option<String>,
    pub check: Option<String>,
    #[fhir_serde(rename = "logMessage")]
    pub log_message: Option<String>,
}

/// Choice of types for the value\[x\] field in StructureMapGroupRuleTargetParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum StructureMapGroupRuleTargetParameterValue {
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRuleTargetParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<StructureMapGroupRuleTargetParameterValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub extends: Option<Id>,
    #[fhir_serde(rename = "typeMode")]
    pub type_mode: Code,
    pub documentation: Option<String>,
    pub input: Option<Vec<StructureMapGroupInput>>,
    pub rule: Option<Vec<StructureMapGroupRule>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRuleTarget {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Option<Id>,
    #[fhir_serde(rename = "contextType")]
    pub context_type: Option<Code>,
    pub element: Option<String>,
    pub variable: Option<Id>,
    #[fhir_serde(rename = "listMode")]
    pub list_mode: Option<Vec<Code>>,
    #[fhir_serde(rename = "listRuleId")]
    pub list_rule_id: Option<Id>,
    pub transform: Option<Code>,
    pub parameter: Option<Vec<StructureMapGroupRuleTargetParameter>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub source: Option<Vec<StructureMapGroupRuleSource>>,
    pub target: Option<Vec<StructureMapGroupRuleTarget>>,
    pub rule: Option<Vec<StructureMapGroupRule>>,
    pub dependent: Option<Vec<StructureMapGroupRuleDependent>>,
    pub documentation: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Subscription {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub contact: Option<Vec<ContactPoint>>,
    pub end: Option<Instant>,
    pub reason: String,
    pub criteria: String,
    pub error: Option<String>,
    pub channel: SubscriptionChannel,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionChannel {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub endpoint: Option<Url>,
    pub payload: Option<Code>,
    pub header: Option<Vec<String>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionStatus {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Option<Code>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(rename = "eventsSinceSubscriptionStart")]
    pub events_since_subscription_start: Option<String>,
    #[fhir_serde(rename = "notificationEvent")]
    pub notification_event: Option<Vec<SubscriptionStatusNotificationEvent>>,
    pub subscription: Reference,
    pub topic: Option<Canonical>,
    pub error: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionStatusNotificationEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "eventNumber")]
    pub event_number: String,
    pub timestamp: Option<Instant>,
    pub focus: Option<Reference>,
    #[fhir_serde(rename = "additionalContext")]
    pub additional_context: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicNotificationShape {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub resource: Uri,
    pub include: Option<Vec<String>>,
    #[fhir_serde(rename = "revInclude")]
    pub rev_include: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicEventTrigger {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub event: CodeableConcept,
    pub resource: Uri,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicResourceTrigger {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub resource: Uri,
    #[fhir_serde(rename = "supportedInteraction")]
    pub supported_interaction: Option<Vec<Code>>,
    #[fhir_serde(rename = "queryCriteria")]
    pub query_criteria: Option<SubscriptionTopicResourceTriggerQueryCriteria>,
    #[fhir_serde(rename = "fhirPathCriteria")]
    pub fhir_path_criteria: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopic {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[fhir_serde(rename = "resourceTrigger")]
    pub resource_trigger: Option<Vec<SubscriptionTopicResourceTrigger>>,
    #[fhir_serde(rename = "eventTrigger")]
    pub event_trigger: Option<Vec<SubscriptionTopicEventTrigger>>,
    #[fhir_serde(rename = "canFilterBy")]
    pub can_filter_by: Option<Vec<SubscriptionTopicCanFilterBy>>,
    #[fhir_serde(rename = "notificationShape")]
    pub notification_shape: Option<Vec<SubscriptionTopicNotificationShape>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicResourceTriggerQueryCriteria {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub previous: Option<String>,
    #[fhir_serde(rename = "resultForCreate")]
    pub result_for_create: Option<Code>,
    pub current: Option<String>,
    #[fhir_serde(rename = "resultForDelete")]
    pub result_for_delete: Option<Code>,
    #[fhir_serde(rename = "requireBoth")]
    pub require_both: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicCanFilterBy {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub resource: Option<Uri>,
    #[fhir_serde(rename = "filterParameter")]
    pub filter_parameter: String,
    #[fhir_serde(rename = "filterDefinition")]
    pub filter_definition: Option<Uri>,
    pub modifier: Option<Vec<Code>>,
}


/// Choice of types for the substance\[x\] field in SubstanceIngredient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SubstanceIngredientSubstance {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "substanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "substanceReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceIngredient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Ratio>,
    #[fhir_serde(flatten)]
    pub substance: Option<SubstanceIngredientSubstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub expiry: Option<DateTime>,
    pub quantity: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Substance {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub description: Option<String>,
    pub instance: Option<Vec<SubstanceInstance>>,
    pub ingredient: Option<Vec<SubstanceIngredient>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub status: Option<CodeableConcept>,
    pub classification: Option<Vec<CodeableConcept>>,
    pub domain: Option<CodeableConcept>,
    pub grade: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "informationSource")]
    pub information_source: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub manufacturer: Option<Vec<Reference>>,
    pub supplier: Option<Vec<Reference>>,
    pub moiety: Option<Vec<SubstanceDefinitionMoiety>>,
    pub property: Option<Vec<SubstanceDefinitionProperty>>,
    #[fhir_serde(rename = "molecularWeight")]
    pub molecular_weight: Option<Vec<SubstanceDefinitionMolecularWeight>>,
    pub structure: Option<SubstanceDefinitionStructure>,
    pub code: Option<Vec<SubstanceDefinitionCode>>,
    pub name: Option<Vec<SubstanceDefinitionName>>,
    pub relationship: Option<Vec<SubstanceDefinitionRelationship>>,
    #[fhir_serde(rename = "sourceMaterial")]
    pub source_material: Option<SubstanceDefinitionSourceMaterial>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionNameOfficial {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub authority: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    pub date: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionSourceMaterial {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub genus: Option<CodeableConcept>,
    pub species: Option<CodeableConcept>,
    pub part: Option<CodeableConcept>,
    #[fhir_serde(rename = "countryOfOrigin")]
    pub country_of_origin: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    pub preferred: Option<Boolean>,
    pub language: Option<Vec<CodeableConcept>>,
    pub domain: Option<Vec<CodeableConcept>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub synonym: Option<Vec<SubstanceDefinitionName>>,
    pub translation: Option<Vec<SubstanceDefinitionName>>,
    pub official: Option<Vec<SubstanceDefinitionNameOfficial>>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionStructureRepresentation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub representation: Option<String>,
    pub format: Option<CodeableConcept>,
    pub document: Option<Reference>,
}

/// Choice of types for the amount\[x\] field in SubstanceDefinitionMoiety
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SubstanceDefinitionMoietyAmount {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionMoiety {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    pub stereochemistry: Option<CodeableConcept>,
    #[fhir_serde(rename = "opticalActivity")]
    pub optical_activity: Option<CodeableConcept>,
    #[fhir_serde(rename = "molecularFormula")]
    pub molecular_formula: Option<String>,
    #[fhir_serde(flatten)]
    pub amount: Option<SubstanceDefinitionMoietyAmount>,
    #[fhir_serde(rename = "measurementType")]
    pub measurement_type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionStructure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub stereochemistry: Option<CodeableConcept>,
    #[fhir_serde(rename = "opticalActivity")]
    pub optical_activity: Option<CodeableConcept>,
    #[fhir_serde(rename = "molecularFormula")]
    pub molecular_formula: Option<String>,
    #[fhir_serde(rename = "molecularFormulaByMoiety")]
    pub molecular_formula_by_moiety: Option<String>,
    #[fhir_serde(rename = "molecularWeight")]
    pub molecular_weight: Option<SubstanceDefinitionMolecularWeight>,
    pub technique: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "sourceDocument")]
    pub source_document: Option<Vec<Reference>>,
    pub representation: Option<Vec<SubstanceDefinitionStructureRepresentation>>,
}

/// Choice of types for the value\[x\] field in SubstanceDefinitionProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SubstanceDefinitionPropertyValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<SubstanceDefinitionPropertyValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionCode {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    pub note: Option<Vec<Annotation>>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionMolecularWeight {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub method: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub amount: Quantity,
}

/// Choice of types for the substanceDefinition\[x\] field in SubstanceDefinitionRelationship
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SubstanceDefinitionRelationshipSubstanceDefinition {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "substanceDefinitionReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "substanceDefinitionCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the amount\[x\] field in SubstanceDefinitionRelationship
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SubstanceDefinitionRelationshipAmount {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "amountRatio")]
    Ratio(Ratio),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceDefinitionRelationship {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub substance_definition: Option<SubstanceDefinitionRelationshipSubstanceDefinition>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(rename = "isDefining")]
    pub is_defining: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub amount: Option<SubstanceDefinitionRelationshipAmount>,
    #[fhir_serde(rename = "ratioHighLimitAmount")]
    pub ratio_high_limit_amount: Option<Ratio>,
    pub comparator: Option<CodeableConcept>,
    pub source: Option<Vec<Reference>>,
}


/// Choice of types for the occurrence\[x\] field in SupplyDelivery
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SupplyDeliveryOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SupplyDelivery {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Option<Code>,
    pub patient: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "suppliedItem")]
    pub supplied_item: Option<SupplyDeliverySuppliedItem>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<SupplyDeliveryOccurrence>,
    pub supplier: Option<Reference>,
    pub destination: Option<Reference>,
    pub receiver: Option<Vec<Reference>>,
}

/// Choice of types for the item\[x\] field in SupplyDeliverySuppliedItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SupplyDeliverySuppliedItemItem {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SupplyDeliverySuppliedItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(flatten)]
    pub item: Option<SupplyDeliverySuppliedItemItem>,
}


/// Choice of types for the item\[x\] field in SupplyRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SupplyRequestItem {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "itemReference")]
    Reference(Reference),
}

/// Choice of types for the occurrence\[x\] field in SupplyRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SupplyRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SupplyRequest {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<CodeableConcept>,
    pub priority: Option<Code>,
    #[fhir_serde(flatten)]
    pub item: Option<SupplyRequestItem>,
    pub quantity: Quantity,
    pub parameter: Option<Vec<SupplyRequestParameter>>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<SupplyRequestOccurrence>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub supplier: Option<Vec<Reference>>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "deliverFrom")]
    pub deliver_from: Option<Reference>,
    #[fhir_serde(rename = "deliverTo")]
    pub deliver_to: Option<Reference>,
}

/// Choice of types for the value\[x\] field in SupplyRequestParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum SupplyRequestParameterValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SupplyRequestParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<SupplyRequestParameterValue>,
}


/// Choice of types for the value\[x\] field in TaskOutput
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum TaskOutputValue {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TaskOutput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<TaskOutputValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TaskRestriction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub repetitions: Option<PositiveInt>,
    pub period: Option<Period>,
    pub recipient: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Task {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Canonical>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Uri>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    #[fhir_serde(rename = "businessStatus")]
    pub business_status: Option<CodeableConcept>,
    pub intent: Code,
    pub priority: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub focus: Option<Reference>,
    #[fhir_serde(rename = "for")]
    pub r#for: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "executionPeriod")]
    pub execution_period: Option<Period>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    #[fhir_serde(rename = "lastModified")]
    pub last_modified: Option<DateTime>,
    pub requester: Option<Reference>,
    #[fhir_serde(rename = "performerType")]
    pub performer_type: Option<Vec<CodeableConcept>>,
    pub owner: Option<Reference>,
    pub location: Option<Reference>,
    #[fhir_serde(rename = "reasonCode")]
    pub reason_code: Option<CodeableConcept>,
    #[fhir_serde(rename = "reasonReference")]
    pub reason_reference: Option<Reference>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
    pub restriction: Option<TaskRestriction>,
    pub input: Option<Vec<TaskInput>>,
    pub output: Option<Vec<TaskOutput>>,
}

/// Choice of types for the value\[x\] field in TaskInput
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum TaskInputValue {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TaskInput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<TaskInputValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilities {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub kind: Code,
    pub software: Option<TerminologyCapabilitiesSoftware>,
    pub implementation: Option<TerminologyCapabilitiesImplementation>,
    #[fhir_serde(rename = "lockedDate")]
    pub locked_date: Option<Boolean>,
    #[fhir_serde(rename = "codeSystem")]
    pub code_system: Option<Vec<TerminologyCapabilitiesCodeSystem>>,
    pub expansion: Option<TerminologyCapabilitiesExpansion>,
    #[fhir_serde(rename = "codeSearch")]
    pub code_search: Option<Code>,
    #[fhir_serde(rename = "validateCode")]
    pub validate_code: Option<TerminologyCapabilitiesValidateCode>,
    pub translation: Option<TerminologyCapabilitiesTranslation>,
    pub closure: Option<TerminologyCapabilitiesClosure>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesExpansionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesClosure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub translation: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesCodeSystemVersion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<String>,
    #[fhir_serde(rename = "isDefault")]
    pub is_default: Option<Boolean>,
    pub compositional: Option<Boolean>,
    pub language: Option<Vec<Code>>,
    pub filter: Option<Vec<TerminologyCapabilitiesCodeSystemVersionFilter>>,
    pub property: Option<Vec<Code>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesExpansion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub hierarchical: Option<Boolean>,
    pub paging: Option<Boolean>,
    pub incomplete: Option<Boolean>,
    pub parameter: Option<Vec<TerminologyCapabilitiesExpansionParameter>>,
    #[fhir_serde(rename = "textFilter")]
    pub text_filter: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesCodeSystemVersionFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub op: Option<Vec<Code>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesTranslation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "needsMap")]
    pub needs_map: Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesValidateCode {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub translations: Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesImplementation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub url: Option<Url>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesCodeSystem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Option<Canonical>,
    pub version: Option<Vec<TerminologyCapabilitiesCodeSystemVersion>>,
    pub subsumption: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesSoftware {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub version: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub uri: Uri,
    pub display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportSetupActionOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    pub message: Option<Markdown>,
    pub detail: Option<Uri>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportTeardownAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestReportSetupActionOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportSetupAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestReportSetupActionOperation>,
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportSetupActionAssert {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    pub message: Option<Markdown>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportTest {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub action: Option<Vec<TestReportTestAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportSetup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestReportSetupAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReport {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    pub status: Code,
    #[fhir_serde(rename = "testScript")]
    pub test_script: Reference,
    pub result: Code,
    pub score: Option<Decimal>,
    pub tester: Option<String>,
    pub issued: Option<DateTime>,
    pub participant: Option<Vec<TestReportParticipant>>,
    pub setup: Option<TestReportSetup>,
    pub test: Option<Vec<TestReportTest>>,
    pub teardown: Option<TestReportTeardown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportTestAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestReportSetupActionOperation>,
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportTeardown {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestReportTeardownAction>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupActionOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Coding>,
    pub resource: Option<Code>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub accept: Option<Code>,
    #[fhir_serde(rename = "contentType")]
    pub content_type: Option<Code>,
    pub destination: Option<Integer>,
    #[fhir_serde(rename = "encodeRequestUrl")]
    pub encode_request_url: Boolean,
    pub method: Option<Code>,
    pub origin: Option<Integer>,
    pub params: Option<String>,
    #[fhir_serde(rename = "requestHeader")]
    pub request_header: Option<Vec<TestScriptSetupActionOperationRequestHeader>>,
    #[fhir_serde(rename = "requestId")]
    pub request_id: Option<Id>,
    #[fhir_serde(rename = "responseId")]
    pub response_id: Option<Id>,
    #[fhir_serde(rename = "sourceId")]
    pub source_id: Option<Id>,
    #[fhir_serde(rename = "targetId")]
    pub target_id: Option<Id>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptMetadata {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub link: Option<Vec<TestScriptMetadataLink>>,
    pub capability: Option<Vec<TestScriptMetadataCapability>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptTeardownAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestScriptSetupActionOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptTest {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub action: Option<Vec<TestScriptTestAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptTestAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupActionAssert {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub direction: Option<Code>,
    #[fhir_serde(rename = "compareToSourceId")]
    pub compare_to_source_id: Option<String>,
    #[fhir_serde(rename = "compareToSourceExpression")]
    pub compare_to_source_expression: Option<String>,
    #[fhir_serde(rename = "compareToSourcePath")]
    pub compare_to_source_path: Option<String>,
    #[fhir_serde(rename = "contentType")]
    pub content_type: Option<Code>,
    pub expression: Option<String>,
    #[fhir_serde(rename = "headerField")]
    pub header_field: Option<String>,
    #[fhir_serde(rename = "minimumId")]
    pub minimum_id: Option<String>,
    #[fhir_serde(rename = "navigationLinks")]
    pub navigation_links: Option<Boolean>,
    pub operator: Option<Code>,
    pub path: Option<String>,
    #[fhir_serde(rename = "requestMethod")]
    pub request_method: Option<Code>,
    #[fhir_serde(rename = "requestURL")]
    pub request_u_r_l: Option<String>,
    pub resource: Option<Code>,
    pub response: Option<Code>,
    #[fhir_serde(rename = "responseCode")]
    pub response_code: Option<String>,
    #[fhir_serde(rename = "sourceId")]
    pub source_id: Option<Id>,
    #[fhir_serde(rename = "validateProfileId")]
    pub validate_profile_id: Option<Id>,
    pub value: Option<String>,
    #[fhir_serde(rename = "warningOnly")]
    pub warning_only: Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptOrigin {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptMetadataCapability {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub required: Boolean,
    pub validated: Boolean,
    pub description: Option<String>,
    pub origin: Option<Vec<Integer>>,
    pub destination: Option<Integer>,
    pub link: Option<Vec<Uri>>,
    pub capabilities: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScript {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Identifier>,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub origin: Option<Vec<TestScriptOrigin>>,
    pub destination: Option<Vec<TestScriptDestination>>,
    pub metadata: Option<TestScriptMetadata>,
    pub fixture: Option<Vec<TestScriptFixture>>,
    pub profile: Option<Vec<Reference>>,
    pub variable: Option<Vec<TestScriptVariable>>,
    pub setup: Option<TestScriptSetup>,
    pub test: Option<Vec<TestScriptTest>>,
    pub teardown: Option<TestScriptTeardown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptMetadataLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestScriptSetupAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptDestination {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptTeardown {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestScriptTeardownAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptFixture {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub autocreate: Boolean,
    pub autodelete: Boolean,
    pub resource: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupActionOperationRequestHeader {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub field: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptVariable {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(rename = "defaultValue")]
    pub default_value: Option<String>,
    pub description: Option<String>,
    pub expression: Option<String>,
    #[fhir_serde(rename = "headerField")]
    pub header_field: Option<String>,
    pub hint: Option<String>,
    pub path: Option<String>,
    #[fhir_serde(rename = "sourceId")]
    pub source_id: Option<Id>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetExpansion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Uri>,
    pub timestamp: DateTime,
    pub total: Option<Integer>,
    pub offset: Option<Integer>,
    pub parameter: Option<Vec<ValueSetExpansionParameter>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetComposeIncludeConceptDesignation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<Code>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Coding>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetCompose {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "lockedDate")]
    pub locked_date: Option<Date>,
    pub inactive: Option<Boolean>,
    pub include: Option<Vec<ValueSetComposeInclude>>,
    pub exclude: Option<Vec<ValueSetComposeInclude>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetExpansionContains {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    #[fhir_serde(rename = "abstract")]
    pub r#abstract: Option<Boolean>,
    pub inactive: Option<Boolean>,
    pub version: Option<String>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetComposeInclude {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    pub version: Option<String>,
    pub concept: Option<Vec<ValueSetComposeIncludeConcept>>,
    pub filter: Option<Vec<ValueSetComposeIncludeFilter>>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Vec<Canonical>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetComposeIncludeFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Code,
    pub op: Code,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSet {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub immutable: Option<Boolean>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub compose: Option<ValueSetCompose>,
    pub expansion: Option<ValueSetExpansion>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetComposeIncludeConcept {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub display: Option<String>,
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
}

/// Choice of types for the value\[x\] field in ValueSetExpansionParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ValueSetExpansionParameterValue {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetExpansionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(flatten)]
    pub value: Option<ValueSetExpansionParameterValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct VerificationResultValidator {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub organization: Reference,
    #[fhir_serde(rename = "identityCertificate")]
    pub identity_certificate: Option<String>,
    #[fhir_serde(rename = "attestationSignature")]
    pub attestation_signature: Option<Signature>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct VerificationResultPrimarySource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub who: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "communicationMethod")]
    pub communication_method: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "validationStatus")]
    pub validation_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "validationDate")]
    pub validation_date: Option<DateTime>,
    #[fhir_serde(rename = "canPushUpdates")]
    pub can_push_updates: Option<CodeableConcept>,
    #[fhir_serde(rename = "pushTypeAvailable")]
    pub push_type_available: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct VerificationResultAttestation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub who: Option<Reference>,
    #[fhir_serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    #[fhir_serde(rename = "communicationMethod")]
    pub communication_method: Option<CodeableConcept>,
    pub date: Option<Date>,
    #[fhir_serde(rename = "sourceIdentityCertificate")]
    pub source_identity_certificate: Option<String>,
    #[fhir_serde(rename = "proxyIdentityCertificate")]
    pub proxy_identity_certificate: Option<String>,
    #[fhir_serde(rename = "proxySignature")]
    pub proxy_signature: Option<Signature>,
    #[fhir_serde(rename = "sourceSignature")]
    pub source_signature: Option<Signature>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct VerificationResult {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Option<Vec<Reference>>,
    #[fhir_serde(rename = "targetLocation")]
    pub target_location: Option<Vec<String>>,
    pub need: Option<CodeableConcept>,
    pub status: Code,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    #[fhir_serde(rename = "validationType")]
    pub validation_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "validationProcess")]
    pub validation_process: Option<Vec<CodeableConcept>>,
    pub frequency: Option<Timing>,
    #[fhir_serde(rename = "lastPerformed")]
    pub last_performed: Option<DateTime>,
    #[fhir_serde(rename = "nextScheduled")]
    pub next_scheduled: Option<Date>,
    #[fhir_serde(rename = "failureAction")]
    pub failure_action: Option<CodeableConcept>,
    #[fhir_serde(rename = "primarySource")]
    pub primary_source: Option<Vec<VerificationResultPrimarySource>>,
    pub attestation: Option<VerificationResultAttestation>,
    pub validator: Option<Vec<VerificationResultValidator>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct VisionPrescription {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub created: DateTime,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "dateWritten")]
    pub date_written: DateTime,
    pub prescriber: Reference,
    #[fhir_serde(rename = "lensSpecification")]
    pub lens_specification: Option<Vec<VisionPrescriptionLensSpecification>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct VisionPrescriptionLensSpecification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub product: CodeableConcept,
    pub eye: Code,
    pub sphere: Option<Decimal>,
    pub cylinder: Option<Decimal>,
    pub axis: Option<Integer>,
    pub prism: Option<Vec<VisionPrescriptionLensSpecificationPrism>>,
    pub add: Option<Decimal>,
    pub power: Option<Decimal>,
    #[fhir_serde(rename = "backCurve")]
    pub back_curve: Option<Decimal>,
    pub diameter: Option<Decimal>,
    pub duration: Option<Quantity>,
    pub color: Option<String>,
    pub brand: Option<String>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct VisionPrescriptionLensSpecificationPrism {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub amount: Decimal,
    pub base: Code,
}


/// Choice of types for the value\[x\] field in ViewDefinitionConstant
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ViewDefinitionConstantValue {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "valueUuid")]
    Uuid(Uuid),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ViewDefinitionConstant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(flatten)]
    pub value: Option<ViewDefinitionConstantValue>,
}

/// Choice of types for the versionAlgorithm\[x\] field in ViewDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ViewDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ViewDefinition {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[fhir_serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<ViewDefinitionVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    pub resource: Code,
    pub profile: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "fhirVersion")]
    pub fhir_version: Option<Vec<Code>>,
    pub constant: Option<Vec<ViewDefinitionConstant>>,
    pub select: Option<Vec<ViewDefinitionSelect>>,
    #[fhir_serde(rename = "where")]
    pub r#where: Option<Vec<ViewDefinitionWhere>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ViewDefinitionSelectColumn {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub name: String,
    pub description: Option<Markdown>,
    pub collection: Option<Boolean>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Uri>,
    pub tag: Option<Vec<ViewDefinitionSelectColumnTag>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ViewDefinitionSelectColumnTag {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ViewDefinitionWhere {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ViewDefinitionSelect {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub column: Option<Vec<ViewDefinitionSelectColumn>>,
    pub select: Option<Vec<ViewDefinitionSelect>>,
    #[fhir_serde(rename = "forEach")]
    pub for_each: Option<String>,
    #[fhir_serde(rename = "forEachOrNull")]
    pub for_each_or_null: Option<String>,
    #[fhir_serde(rename = "unionAll")]
    pub union_all: Option<Vec<ViewDefinitionSelect>>,
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, FhirPath)]
#[serde(tag = "resourceType")]
pub enum Resource {
    Account(Account),
    ActivityDefinition(ActivityDefinition),
    AdministrableProductDefinition(AdministrableProductDefinition),
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
    Citation(Citation),
    Claim(Claim),
    ClaimResponse(ClaimResponse),
    ClinicalImpression(ClinicalImpression),
    ClinicalUseDefinition(ClinicalUseDefinition),
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
    Encounter(Encounter),
    Endpoint(Endpoint),
    EnrollmentRequest(EnrollmentRequest),
    EnrollmentResponse(EnrollmentResponse),
    EpisodeOfCare(EpisodeOfCare),
    EventDefinition(EventDefinition),
    Evidence(Evidence),
    EvidenceReport(EvidenceReport),
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
    Ingredient(Ingredient),
    InsurancePlan(InsurancePlan),
    Invoice(Invoice),
    Library(Library),
    Linkage(Linkage),
    List(List),
    Location(Location),
    ManufacturedItemDefinition(ManufacturedItemDefinition),
    Measure(Measure),
    MeasureReport(MeasureReport),
    Media(Media),
    Medication(Medication),
    MedicationAdministration(MedicationAdministration),
    MedicationDispense(MedicationDispense),
    MedicationKnowledge(MedicationKnowledge),
    MedicationRequest(MedicationRequest),
    MedicationStatement(MedicationStatement),
    MedicinalProductDefinition(MedicinalProductDefinition),
    MessageDefinition(MessageDefinition),
    MessageHeader(MessageHeader),
    MolecularSequence(MolecularSequence),
    NamingSystem(NamingSystem),
    NutritionOrder(NutritionOrder),
    NutritionProduct(NutritionProduct),
    Observation(Observation),
    ObservationDefinition(ObservationDefinition),
    OperationDefinition(OperationDefinition),
    OperationOutcome(OperationOutcome),
    Organization(Organization),
    OrganizationAffiliation(OrganizationAffiliation),
    PackagedProductDefinition(PackagedProductDefinition),
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
    RegulatedAuthorization(RegulatedAuthorization),
    RelatedPerson(RelatedPerson),
    RequestGroup(RequestGroup),
    ResearchDefinition(ResearchDefinition),
    ResearchElementDefinition(ResearchElementDefinition),
    ResearchStudy(ResearchStudy),
    ResearchSubject(ResearchSubject),
    RiskAssessment(RiskAssessment),
    Schedule(Schedule),
    SearchParameter(SearchParameter),
    ServiceRequest(ServiceRequest),
    Slot(Slot),
    Specimen(Specimen),
    SpecimenDefinition(SpecimenDefinition),
    StructureDefinition(StructureDefinition),
    StructureMap(StructureMap),
    Subscription(Subscription),
    SubscriptionStatus(SubscriptionStatus),
    SubscriptionTopic(SubscriptionTopic),
    Substance(Substance),
    SubstanceDefinition(SubstanceDefinition),
    SupplyDelivery(SupplyDelivery),
    SupplyRequest(SupplyRequest),
    Task(Task),
    TerminologyCapabilities(TerminologyCapabilities),
    TestReport(TestReport),
    TestScript(TestScript),
    ValueSet(ValueSet),
    VerificationResult(VerificationResult),
    VisionPrescription(VisionPrescription),
    ViewDefinition(ViewDefinition),
}


// --- From<T> Implementations for Element<T, Extension> ---
impl From<bool> for Element<bool, Extension> {
    fn from(value: bool) -> Self {
        Self { value: Some(value), ..Default::default() }
    }
}
impl From<std::primitive::i32> for Element<std::primitive::i32, Extension> {
    fn from(value: std::primitive::i32) -> Self {
        Self { value: Some(value), ..Default::default() }
    }
}
impl From<std::string::String> for Element<std::string::String, Extension> {
    fn from(value: std::string::String) -> Self {
        Self { value: Some(value), ..Default::default() }
    }
}
// --- End From<T> Implementations ---
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Address {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Code>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: Option<String>,
    pub line: Option<Vec<String>>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub state: Option<String>,
    #[fhir_serde(rename = "postalCode")]
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Age {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


/// Choice of types for the author\[x\] field in Annotation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum AnnotationAuthor {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "authorReference")]
    Reference(Reference),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "authorString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Annotation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub author: Option<AnnotationAuthor>,
    pub time: Option<DateTime>,
    pub text: Markdown,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Attachment {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "contentType")]
    pub content_type: Option<Code>,
    pub language: Option<Code>,
    pub data: Option<Base64Binary>,
    pub url: Option<Url>,
    pub size: Option<UnsignedInt>,
    pub hash: Option<Base64Binary>,
    pub title: Option<String>,
    pub creation: Option<DateTime>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CodeableConcept {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub coding: Option<Vec<Coding>>,
    pub text: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CodeableReference {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub concept: Option<CodeableConcept>,
    pub reference: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Coding {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    pub version: Option<String>,
    pub code: Option<Code>,
    pub display: Option<String>,
    #[fhir_serde(rename = "userSelected")]
    pub user_selected: Option<Boolean>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContactDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub telecom: Option<Vec<ContactPoint>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ContactPoint {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub system: Option<Code>,
    pub value: Option<String>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Code>,
    pub rank: Option<PositiveInt>,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Contributor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub name: String,
    pub contact: Option<Vec<ContactDetail>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Count {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DataRequirementSort {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub direction: Code,
}

/// Choice of types for the subject\[x\] field in DataRequirement
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DataRequirementSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DataRequirement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub profile: Option<Vec<Canonical>>,
    #[fhir_serde(flatten)]
    pub subject: Option<DataRequirementSubject>,
    #[fhir_serde(rename = "mustSupport")]
    pub must_support: Option<Vec<String>>,
    #[fhir_serde(rename = "codeFilter")]
    pub code_filter: Option<Vec<DataRequirementCodeFilter>>,
    #[fhir_serde(rename = "dateFilter")]
    pub date_filter: Option<Vec<DataRequirementDateFilter>>,
    pub limit: Option<PositiveInt>,
    pub sort: Option<Vec<DataRequirementSort>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DataRequirementCodeFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    #[fhir_serde(rename = "searchParam")]
    pub search_param: Option<String>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub code: Option<Vec<Coding>>,
}

/// Choice of types for the value\[x\] field in DataRequirementDateFilter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DataRequirementDateFilterValue {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "valueDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DataRequirementDateFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    #[fhir_serde(rename = "searchParam")]
    pub search_param: Option<String>,
    #[fhir_serde(flatten)]
    pub value: Option<DataRequirementDateFilterValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Distance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


/// Choice of types for the asNeeded\[x\] field in Dosage
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DosageAsNeeded {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Dosage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<Integer>,
    pub text: Option<String>,
    #[fhir_serde(rename = "additionalInstruction")]
    pub additional_instruction: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "patientInstruction")]
    pub patient_instruction: Option<String>,
    pub timing: Option<Timing>,
    #[fhir_serde(flatten)]
    pub as_needed: Option<DosageAsNeeded>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    #[fhir_serde(rename = "doseAndRate")]
    pub dose_and_rate: Option<Vec<DosageDoseAndRate>>,
    #[fhir_serde(rename = "maxDosePerPeriod")]
    pub max_dose_per_period: Option<Ratio>,
    #[fhir_serde(rename = "maxDosePerAdministration")]
    pub max_dose_per_administration: Option<Quantity>,
    #[fhir_serde(rename = "maxDosePerLifetime")]
    pub max_dose_per_lifetime: Option<Quantity>,
}

/// Choice of types for the dose\[x\] field in DosageDoseAndRate
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DosageDoseAndRateDose {
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "doseRange")]
    Range(Range),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "doseQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the rate\[x\] field in DosageDoseAndRate
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum DosageDoseAndRateRate {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "rateRatio")]
    Ratio(Ratio),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "rateRange")]
    Range(Range),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "rateQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DosageDoseAndRate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub dose: Option<DosageDoseAndRateDose>,
    #[fhir_serde(flatten)]
    pub rate: Option<DosageDoseAndRateRate>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Duration {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionBase {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub min: UnsignedInt,
    pub max: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionBinding {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub strength: Code,
    pub description: Option<String>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionMapping {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub identity: Id,
    pub language: Option<Code>,
    pub map: String,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionConstraint {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub key: Id,
    pub requirements: Option<String>,
    pub severity: Code,
    pub human: String,
    pub expression: Option<String>,
    pub xpath: Option<String>,
    pub source: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionSlicing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub discriminator: Option<Vec<ElementDefinitionSlicingDiscriminator>>,
    pub description: Option<String>,
    pub ordered: Option<Boolean>,
    pub rules: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionSlicingDiscriminator {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub path: String,
}

/// Choice of types for the defaultValue\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ElementDefinitionDefaultValue {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "defaultValueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "defaultValueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "defaultValueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "defaultValueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "defaultValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "defaultValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "defaultValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "defaultValueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "defaultValueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "defaultValueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "defaultValueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "defaultValueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "defaultValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "defaultValueString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "defaultValueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "defaultValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "defaultValueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "defaultValueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "defaultValueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "defaultValueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "defaultValueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "defaultValueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "defaultValueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "defaultValueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the CodeableReference type.
    #[fhir_serde(rename = "defaultValueCodeableReference")]
    CodeableReference(CodeableReference),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "defaultValueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "defaultValueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "defaultValueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "defaultValueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "defaultValueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "defaultValueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "defaultValueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "defaultValueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "defaultValuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "defaultValueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "defaultValueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "defaultValueRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "defaultValueRatioRange")]
    RatioRange(RatioRange),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "defaultValueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "defaultValueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "defaultValueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "defaultValueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "defaultValueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "defaultValueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "defaultValueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "defaultValueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "defaultValueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "defaultValueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "defaultValueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "defaultValueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "defaultValueDosage")]
    Dosage(Dosage),
}

/// Choice of types for the fixed\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ElementDefinitionFixed {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "fixedBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "fixedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "fixedCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "fixedCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "fixedDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "fixedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "fixedDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "fixedId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "fixedInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "fixedInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "fixedMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "fixedOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "fixedPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "fixedString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "fixedTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "fixedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "fixedUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "fixedUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "fixedUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "fixedAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "fixedAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "fixedAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "fixedAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "fixedCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the CodeableReference type.
    #[fhir_serde(rename = "fixedCodeableReference")]
    CodeableReference(CodeableReference),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "fixedCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "fixedContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "fixedCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "fixedDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "fixedDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "fixedHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "fixedIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "fixedMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "fixedPeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "fixedQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "fixedRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "fixedRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "fixedRatioRange")]
    RatioRange(RatioRange),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "fixedReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "fixedSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "fixedSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "fixedTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "fixedContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "fixedContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "fixedDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "fixedExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "fixedParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "fixedRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "fixedTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "fixedUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "fixedDosage")]
    Dosage(Dosage),
}

/// Choice of types for the pattern\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ElementDefinitionPattern {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "patternBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "patternBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "patternCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "patternCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "patternDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "patternDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "patternDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "patternId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "patternInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "patternInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "patternMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "patternOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "patternPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "patternString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "patternTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "patternUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "patternUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "patternUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "patternUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "patternAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "patternAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "patternAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "patternAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "patternCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the CodeableReference type.
    #[fhir_serde(rename = "patternCodeableReference")]
    CodeableReference(CodeableReference),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "patternCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "patternContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "patternCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "patternDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "patternDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "patternHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "patternIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "patternMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "patternPeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "patternQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "patternRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "patternRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "patternRatioRange")]
    RatioRange(RatioRange),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "patternReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "patternSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "patternSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "patternTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "patternContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "patternContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "patternDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "patternExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "patternParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "patternRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "patternTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "patternUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "patternDosage")]
    Dosage(Dosage),
}

/// Choice of types for the minValue\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ElementDefinitionMinValue {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "minValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "minValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "minValueInstant")]
    Instant(Instant),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "minValueTime")]
    Time(Time),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "minValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "minValueInteger")]
    Integer(Integer),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "minValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "minValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "minValueQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the maxValue\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ElementDefinitionMaxValue {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "maxValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "maxValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "maxValueInstant")]
    Instant(Instant),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "maxValueTime")]
    Time(Time),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "maxValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "maxValueInteger")]
    Integer(Integer),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "maxValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "maxValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "maxValueQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub representation: Option<Vec<Code>>,
    #[fhir_serde(rename = "sliceName")]
    pub slice_name: Option<String>,
    #[fhir_serde(rename = "sliceIsConstraining")]
    pub slice_is_constraining: Option<Boolean>,
    pub label: Option<String>,
    pub code: Option<Vec<Coding>>,
    pub slicing: Option<ElementDefinitionSlicing>,
    pub short: Option<String>,
    pub definition: Option<Markdown>,
    pub comment: Option<Markdown>,
    pub requirements: Option<Markdown>,
    pub alias: Option<Vec<String>>,
    pub min: Option<UnsignedInt>,
    pub max: Option<String>,
    pub base: Option<ElementDefinitionBase>,
    #[fhir_serde(rename = "contentReference")]
    pub content_reference: Option<Uri>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<ElementDefinitionType>>,
    #[fhir_serde(flatten)]
    pub default_value: Option<ElementDefinitionDefaultValue>,
    #[fhir_serde(rename = "meaningWhenMissing")]
    pub meaning_when_missing: Option<Markdown>,
    #[fhir_serde(rename = "orderMeaning")]
    pub order_meaning: Option<String>,
    #[fhir_serde(flatten)]
    pub fixed: Option<ElementDefinitionFixed>,
    #[fhir_serde(flatten)]
    pub pattern: Option<ElementDefinitionPattern>,
    pub example: Option<Vec<ElementDefinitionExample>>,
    #[fhir_serde(flatten)]
    pub min_value: Option<ElementDefinitionMinValue>,
    #[fhir_serde(flatten)]
    pub max_value: Option<ElementDefinitionMaxValue>,
    #[fhir_serde(rename = "maxLength")]
    pub max_length: Option<Integer>,
    pub condition: Option<Vec<Id>>,
    pub constraint: Option<Vec<ElementDefinitionConstraint>>,
    #[fhir_serde(rename = "mustSupport")]
    pub must_support: Option<Boolean>,
    #[fhir_serde(rename = "isModifier")]
    pub is_modifier: Option<Boolean>,
    #[fhir_serde(rename = "isModifierReason")]
    pub is_modifier_reason: Option<String>,
    #[fhir_serde(rename = "isSummary")]
    pub is_summary: Option<Boolean>,
    pub binding: Option<ElementDefinitionBinding>,
    pub mapping: Option<Vec<ElementDefinitionMapping>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionType {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Uri,
    pub profile: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "targetProfile")]
    pub target_profile: Option<Vec<Canonical>>,
    pub aggregation: Option<Vec<Code>>,
    pub versioning: Option<Code>,
}

/// Choice of types for the value\[x\] field in ElementDefinitionExample
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ElementDefinitionExampleValue {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the CodeableReference type.
    #[fhir_serde(rename = "valueCodeableReference")]
    CodeableReference(CodeableReference),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "valueRatioRange")]
    RatioRange(RatioRange),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionExample {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub label: String,
    #[fhir_serde(flatten)]
    pub value: Option<ElementDefinitionExampleValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Expression {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub name: Option<Id>,
    pub language: Code,
    pub expression: Option<String>,
    pub reference: Option<Uri>,
}


/// Choice of types for the value\[x\] field in Extension
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum ExtensionValue {
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[fhir_serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[fhir_serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[fhir_serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[fhir_serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the CodeableReference type.
    #[fhir_serde(rename = "valueCodeableReference")]
    CodeableReference(CodeableReference),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[fhir_serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[fhir_serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[fhir_serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "valueRatioRange")]
    RatioRange(RatioRange),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[fhir_serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[fhir_serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[fhir_serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[fhir_serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[fhir_serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[fhir_serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[fhir_serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[fhir_serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[fhir_serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Extension {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub url: String,
    #[fhir_serde(flatten)]
    pub value: Option<ExtensionValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct HumanName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Code>,
    pub text: Option<String>,
    pub family: Option<String>,
    pub given: Option<Vec<String>>,
    pub prefix: Option<Vec<String>>,
    pub suffix: Option<Vec<String>>,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Identifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Code>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub system: Option<Uri>,
    pub value: Option<String>,
    pub period: Option<Period>,
    pub assigner: Option<Box<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MarketingStatus {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: Option<CodeableConcept>,
    pub jurisdiction: Option<CodeableConcept>,
    pub status: CodeableConcept,
    #[fhir_serde(rename = "dateRange")]
    pub date_range: Option<Period>,
    #[fhir_serde(rename = "restoreDate")]
    pub restore_date: Option<DateTime>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Meta {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "versionId")]
    pub version_id: Option<Id>,
    #[fhir_serde(rename = "lastUpdated")]
    pub last_updated: Option<Instant>,
    pub source: Option<Uri>,
    pub profile: Option<Vec<Canonical>>,
    pub security: Option<Vec<Coding>>,
    pub tag: Option<Vec<Coding>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Money {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub currency: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Narrative {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub status: Code,
    pub div: Xhtml,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ParameterDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub name: Option<Code>,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub min: Option<Integer>,
    pub max: Option<String>,
    pub documentation: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub profile: Option<Canonical>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Period {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub start: Option<DateTime>,
    pub end: Option<DateTime>,
}


/// Choice of types for the age\[x\] field in Population
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum PopulationAge {
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "ageRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "ageCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Population {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub age: Option<PopulationAge>,
    pub gender: Option<CodeableConcept>,
    pub race: Option<CodeableConcept>,
    #[fhir_serde(rename = "physiologicalCondition")]
    pub physiological_condition: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ProdCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub height: Option<Quantity>,
    pub width: Option<Quantity>,
    pub depth: Option<Quantity>,
    pub weight: Option<Quantity>,
    #[fhir_serde(rename = "nominalVolume")]
    pub nominal_volume: Option<Quantity>,
    #[fhir_serde(rename = "externalDiameter")]
    pub external_diameter: Option<Quantity>,
    pub shape: Option<String>,
    pub color: Option<Vec<String>>,
    pub imprint: Option<Vec<String>>,
    pub image: Option<Vec<Attachment>>,
    pub scoring: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ProductShelfLife {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub period: Quantity,
    #[fhir_serde(rename = "specialPrecautionsForStorage")]
    pub special_precautions_for_storage: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Quantity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Range {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub low: Option<Quantity>,
    pub high: Option<Quantity>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Ratio {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub numerator: Option<Quantity>,
    pub denominator: Option<Quantity>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RatioRange {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "lowNumerator")]
    pub low_numerator: Option<Quantity>,
    #[fhir_serde(rename = "highNumerator")]
    pub high_numerator: Option<Quantity>,
    pub denominator: Option<Quantity>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Reference {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub reference: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Uri>,
    pub identifier: Option<Box<Identifier>>,
    pub display: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RelatedArtifact {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub label: Option<String>,
    pub display: Option<String>,
    pub citation: Option<Markdown>,
    pub url: Option<Url>,
    pub document: Option<Attachment>,
    pub resource: Option<Canonical>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SampledData {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub origin: Quantity,
    pub period: Decimal,
    pub factor: Option<Decimal>,
    #[fhir_serde(rename = "lowerLimit")]
    pub lower_limit: Option<Decimal>,
    #[fhir_serde(rename = "upperLimit")]
    pub upper_limit: Option<Decimal>,
    pub dimensions: PositiveInt,
    pub data: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Signature {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<Coding>>,
    pub when: Instant,
    pub who: Reference,
    #[fhir_serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    #[fhir_serde(rename = "targetFormat")]
    pub target_format: Option<Code>,
    #[fhir_serde(rename = "sigFormat")]
    pub sig_format: Option<Code>,
    pub data: Option<Base64Binary>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Timing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub event: Option<Vec<DateTime>>,
    pub repeat: Option<TimingRepeat>,
    pub code: Option<CodeableConcept>,
}

/// Choice of types for the bounds\[x\] field in TimingRepeat
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum TimingRepeatBounds {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "boundsDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "boundsRange")]
    Range(Range),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "boundsPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TimingRepeat {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub bounds: Option<TimingRepeatBounds>,
    pub count: Option<PositiveInt>,
    #[fhir_serde(rename = "countMax")]
    pub count_max: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    #[fhir_serde(rename = "durationMax")]
    pub duration_max: Option<Decimal>,
    #[fhir_serde(rename = "durationUnit")]
    pub duration_unit: Option<Code>,
    pub frequency: Option<PositiveInt>,
    #[fhir_serde(rename = "frequencyMax")]
    pub frequency_max: Option<PositiveInt>,
    pub period: Option<Decimal>,
    #[fhir_serde(rename = "periodMax")]
    pub period_max: Option<Decimal>,
    #[fhir_serde(rename = "periodUnit")]
    pub period_unit: Option<Code>,
    #[fhir_serde(rename = "dayOfWeek")]
    pub day_of_week: Option<Vec<Code>>,
    #[fhir_serde(rename = "timeOfDay")]
    pub time_of_day: Option<Vec<Time>>,
    pub when: Option<Vec<Code>>,
    pub offset: Option<UnsignedInt>,
}


/// Choice of types for the timing\[x\] field in TriggerDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum TriggerDefinitionTiming {
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "timingReference")]
    Reference(Reference),
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timingDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TriggerDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub name: Option<String>,
    #[fhir_serde(flatten)]
    pub timing: Option<TriggerDefinitionTiming>,
    pub data: Option<Vec<DataRequirement>>,
    pub condition: Option<Expression>,
}


/// Choice of types for the value\[x\] field in UsageContext
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
pub enum UsageContextValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct UsageContext {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Coding,
    #[fhir_serde(flatten)]
    pub value: Option<UsageContextValue>,
}


