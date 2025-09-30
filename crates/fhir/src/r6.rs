use helios_fhir_macro::{FhirPath, FhirSerde};
use serde::{Deserialize, Serialize};

use crate::{DecimalElement, Element};

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AccountGuarantor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub party: Option<Reference>,
    #[fhir_serde(rename = "onHold")]
    pub on_hold: Option<Boolean>,
    pub period: Option<Period>,
    pub account: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AccountProcedure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<PositiveInt>,
    pub code: CodeableReference,
    #[fhir_serde(rename = "dateOfService")]
    pub date_of_service: Option<DateTime>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "packageCode")]
    pub package_code: Option<Vec<CodeableConcept>>,
    pub device: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AccountBalance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub aggregate: Option<CodeableConcept>,
    pub term: Option<CodeableConcept>,
    pub estimate: Option<Boolean>,
    pub amount: Money,
}

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
    #[fhir_serde(rename = "billingStatus")]
    pub billing_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub name: Option<String>,
    pub subject: Option<Vec<Reference>>,
    #[fhir_serde(rename = "servicePeriod")]
    pub service_period: Option<Period>,
    pub covers: Option<Vec<Reference>>,
    pub coverage: Option<Vec<AccountCoverage>>,
    pub owner: Option<Reference>,
    pub description: Option<Markdown>,
    pub guarantor: Option<Vec<AccountGuarantor>>,
    pub diagnosis: Option<Vec<AccountDiagnosis>>,
    pub procedure: Option<Vec<AccountProcedure>>,
    pub parent: Option<Reference>,
    pub currency: Option<CodeableConcept>,
    pub balance: Option<Vec<AccountBalance>>,
    #[fhir_serde(rename = "calculatedAt")]
    pub calculated_at: Option<Instant>,
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
pub struct AccountDiagnosis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<PositiveInt>,
    pub condition: CodeableReference,
    #[fhir_serde(rename = "dateOfDiagnosis")]
    pub date_of_diagnosis: Option<DateTime>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "onAdmission")]
    pub on_admission: Option<Boolean>,
    #[fhir_serde(rename = "packageCode")]
    pub package_code: Option<Vec<CodeableConcept>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in ActivityDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ActivityDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

/// Choice of types for the subject\[x\] field in ActivityDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
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
#[fhir_choice_element(base_name = "timing")]
pub enum ActivityDefinitionTiming {
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "timingDuration")]
    Duration(Duration),
}

/// Choice of types for the asNeeded\[x\] field in ActivityDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "asNeeded")]
pub enum ActivityDefinitionAsNeeded {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the product\[x\] field in ActivityDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "product")]
pub enum ActivityDefinitionProduct {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm,subject,timing,asNeeded,product")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<ActivityDefinitionVersionAlgorithm>,
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
    pub usage: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
    #[fhir_serde(flatten)]
    pub as_needed: Option<ActivityDefinitionAsNeeded>,
    pub location: Option<CodeableReference>,
    pub participant: Option<Vec<ActivityDefinitionParticipant>>,
    #[fhir_serde(flatten)]
    pub product: Option<ActivityDefinitionProduct>,
    pub quantity: Option<Quantity>,
    pub dosage: Option<Vec<Dosage>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "specimenRequirement")]
    pub specimen_requirement: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "observationRequirement")]
    pub observation_requirement: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "observationResultRequirement")]
    pub observation_result_requirement: Option<Vec<Canonical>>,
    pub transform: Option<Canonical>,
    #[fhir_serde(rename = "dynamicValue")]
    pub dynamic_value: Option<Vec<ActivityDefinitionDynamicValue>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ActivityDefinitionParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    #[fhir_serde(rename = "typeCanonical")]
    pub type_canonical: Option<Canonical>,
    #[fhir_serde(rename = "typeReference")]
    pub type_reference: Option<Reference>,
    pub role: Option<CodeableConcept>,
    pub function: Option<CodeableConcept>,
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

/// Choice of types for the versionAlgorithm\[x\] field in ActorDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ActorDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
pub struct ActorDefinition {
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
    pub version_algorithm: Option<ActorDefinitionVersionAlgorithm>,
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
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub documentation: Option<Markdown>,
    pub reference: Option<Vec<Url>>,
    #[fhir_serde(rename = "baseDefinition")]
    pub base_definition: Option<Vec<Canonical>>,
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
pub struct AdministrableProductDefinitionRouteOfAdministrationTargetSpecies {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(rename = "withdrawalPeriod")]
    pub withdrawal_period: Option<
        Vec<AdministrableProductDefinitionRouteOfAdministrationTargetSpeciesWithdrawalPeriod>,
    >,
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
    pub target_species:
        Option<Vec<AdministrableProductDefinitionRouteOfAdministrationTargetSpecies>>,
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
    pub description: Option<Markdown>,
    pub property: Option<Vec<AdministrableProductDefinitionProperty>>,
    #[fhir_serde(rename = "routeOfAdministration")]
    pub route_of_administration: Option<Vec<AdministrableProductDefinitionRouteOfAdministration>>,
}

/// Choice of types for the value\[x\] field in AdministrableProductDefinitionProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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

/// Choice of types for the cause\[x\] field in AdverseEvent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "cause")]
pub enum AdverseEventCause {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "causeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "causePeriod")]
    Period(Period),
}

/// Choice of types for the effect\[x\] field in AdverseEvent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "effect")]
pub enum AdverseEventEffect {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "cause,effect")]
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
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub actuality: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub cause: Option<AdverseEventCause>,
    #[fhir_serde(flatten)]
    pub effect: Option<AdverseEventEffect>,
    pub detected: Option<DateTime>,
    #[fhir_serde(rename = "recordedDate")]
    pub recorded_date: Option<DateTime>,
    #[fhir_serde(rename = "resultingEffect")]
    pub resulting_effect: Option<Vec<CodeableReference>>,
    pub location: Option<Reference>,
    pub seriousness: Option<CodeableConcept>,
    pub outcome: Option<Vec<CodeableConcept>>,
    pub recorder: Option<Reference>,
    pub participant: Option<Vec<AdverseEventParticipant>>,
    pub study: Option<Vec<Reference>>,
    #[fhir_serde(rename = "expectedInResearchStudy")]
    pub expected_in_research_study: Option<Boolean>,
    #[fhir_serde(rename = "suspectEntity")]
    pub suspect_entity: Option<Vec<AdverseEventSuspectEntity>>,
    #[fhir_serde(rename = "contributingFactor")]
    pub contributing_factor: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "preventiveAction")]
    pub preventive_action: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "mitigatingAction")]
    pub mitigating_action: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdverseEventSuspectEntity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub instance: CodeableReference,
    pub causality: Option<AdverseEventSuspectEntityCausality>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdverseEventSuspectEntityCausality {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "assessmentMethod")]
    pub assessment_method: Option<CodeableConcept>,
    #[fhir_serde(rename = "entityRelatedness")]
    pub entity_relatedness: Option<CodeableConcept>,
    pub author: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AdverseEventParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AllergyIntoleranceReaction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: Option<CodeableConcept>,
    pub manifestation: Option<Vec<CodeableReference>>,
    pub description: Option<String>,
    pub onset: Option<DateTime>,
    pub severity: Option<Code>,
    #[fhir_serde(rename = "exposureRoute")]
    pub exposure_route: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the onset\[x\] field in AllergyIntolerance
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "onset")]
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
#[fhir_resource(choice_elements = "onset")]
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
    pub r#type: Option<CodeableConcept>,
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
    #[fhir_serde(rename = "lastReactionOccurrence")]
    pub last_reaction_occurrence: Option<DateTime>,
    pub note: Option<Vec<Annotation>>,
    pub reaction: Option<Vec<AllergyIntoleranceReaction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AppointmentRecurrenceTemplateMonthlyTemplate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "dayOfMonth")]
    pub day_of_month: Option<PositiveInt>,
    #[fhir_serde(rename = "nthWeekOfMonth")]
    pub nth_week_of_month: Option<Coding>,
    #[fhir_serde(rename = "dayOfWeek")]
    pub day_of_week: Option<Coding>,
    #[fhir_serde(rename = "monthInterval")]
    pub month_interval: PositiveInt,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AppointmentParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub actor: Option<Reference>,
    pub required: Option<Boolean>,
    pub status: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AppointmentRecurrenceTemplateYearlyTemplate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "yearInterval")]
    pub year_interval: PositiveInt,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AppointmentRecurrenceTemplate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub timezone: Option<CodeableConcept>,
    #[fhir_serde(rename = "recurrenceType")]
    pub recurrence_type: CodeableConcept,
    #[fhir_serde(rename = "lastOccurrenceDate")]
    pub last_occurrence_date: Option<Date>,
    #[fhir_serde(rename = "occurrenceCount")]
    pub occurrence_count: Option<PositiveInt>,
    #[fhir_serde(rename = "occurrenceDate")]
    pub occurrence_date: Option<Vec<Date>>,
    #[fhir_serde(rename = "weeklyTemplate")]
    pub weekly_template: Option<AppointmentRecurrenceTemplateWeeklyTemplate>,
    #[fhir_serde(rename = "monthlyTemplate")]
    pub monthly_template: Option<AppointmentRecurrenceTemplateMonthlyTemplate>,
    #[fhir_serde(rename = "yearlyTemplate")]
    pub yearly_template: Option<AppointmentRecurrenceTemplateYearlyTemplate>,
    #[fhir_serde(rename = "excludingDate")]
    pub excluding_date: Option<Vec<Date>>,
    #[fhir_serde(rename = "excludingRecurrenceId")]
    pub excluding_recurrence_id: Option<Vec<PositiveInt>>,
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
    #[fhir_serde(rename = "cancellationReason")]
    pub cancellation_reason: Option<CodeableConcept>,
    pub class: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "serviceCategory")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableReference>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "appointmentType")]
    pub appointment_type: Option<CodeableConcept>,
    pub reason: Option<Vec<CodeableReference>>,
    pub priority: Option<CodeableConcept>,
    pub description: Option<String>,
    pub replaces: Option<Vec<Reference>>,
    #[fhir_serde(rename = "virtualService")]
    pub virtual_service: Option<Vec<VirtualServiceDetail>>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[fhir_serde(rename = "previousAppointment")]
    pub previous_appointment: Option<Reference>,
    #[fhir_serde(rename = "originatingAppointment")]
    pub originating_appointment: Option<Reference>,
    pub start: Option<Instant>,
    pub end: Option<Instant>,
    #[fhir_serde(rename = "minutesDuration")]
    pub minutes_duration: Option<PositiveInt>,
    #[fhir_serde(rename = "requestedPeriod")]
    pub requested_period: Option<Vec<Period>>,
    pub slot: Option<Vec<Reference>>,
    pub account: Option<Vec<Reference>>,
    pub created: Option<DateTime>,
    #[fhir_serde(rename = "cancellationDate")]
    pub cancellation_date: Option<DateTime>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "patientInstruction")]
    pub patient_instruction: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub subject: Option<Reference>,
    pub participant: Option<Vec<AppointmentParticipant>>,
    #[fhir_serde(rename = "recurrenceId")]
    pub recurrence_id: Option<PositiveInt>,
    #[fhir_serde(rename = "occurrenceChanged")]
    pub occurrence_changed: Option<Boolean>,
    #[fhir_serde(rename = "recurrenceTemplate")]
    pub recurrence_template: Option<Vec<AppointmentRecurrenceTemplate>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AppointmentRecurrenceTemplateWeeklyTemplate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub monday: Option<Boolean>,
    pub tuesday: Option<Boolean>,
    pub wednesday: Option<Boolean>,
    pub thursday: Option<Boolean>,
    pub friday: Option<Boolean>,
    pub saturday: Option<Boolean>,
    pub sunday: Option<Boolean>,
    #[fhir_serde(rename = "weekInterval")]
    pub week_interval: Option<PositiveInt>,
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
    #[fhir_serde(rename = "proposedNewTime")]
    pub proposed_new_time: Option<Boolean>,
    pub start: Option<Instant>,
    pub end: Option<Instant>,
    #[fhir_serde(rename = "participantType")]
    pub participant_type: Option<Vec<CodeableConcept>>,
    pub actor: Option<Reference>,
    #[fhir_serde(rename = "participantStatus")]
    pub participant_status: Code,
    pub comment: Option<Markdown>,
    pub recurring: Option<Boolean>,
    #[fhir_serde(rename = "occurrenceDate")]
    pub occurrence_date: Option<Date>,
    #[fhir_serde(rename = "recurrenceId")]
    pub recurrence_id: Option<PositiveInt>,
}

/// Choice of types for the artifact\[x\] field in ArtifactAssessment
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "artifact")]
pub enum ArtifactAssessmentArtifact {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "artifactReference")]
    Reference(Reference),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "artifactCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "artifactUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "artifact")]
pub struct ArtifactAssessment {
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
    #[fhir_serde(rename = "citeAs")]
    pub cite_as: Option<Markdown>,
    #[fhir_serde(flatten)]
    pub artifact: Option<ArtifactAssessmentArtifact>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<ArtifactAssessmentRelatesTo>>,
    pub date: Option<DateTime>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    pub content: Option<Vec<ArtifactAssessmentContent>>,
    #[fhir_serde(rename = "workflowStatus")]
    pub workflow_status: Option<Code>,
    pub disposition: Option<Code>,
}

/// Choice of types for the target\[x\] field in ArtifactAssessmentRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "target")]
pub enum ArtifactAssessmentRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "targetMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "target")]
pub struct ArtifactAssessmentRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(flatten)]
    pub target: Option<ArtifactAssessmentRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ArtifactAssessmentContent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub summary: Option<Markdown>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub classifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub author: Option<Vec<Reference>>,
    pub path: Option<Vec<Uri>>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<ArtifactAssessmentRelatesTo>>,
    #[fhir_serde(rename = "freeToShare")]
    pub free_to_share: Option<Boolean>,
    pub component: Option<Vec<ArtifactAssessmentContent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEventOutcome {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Coding,
    pub detail: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEventEntity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub what: Option<Reference>,
    pub role: Option<CodeableConcept>,
    #[fhir_serde(rename = "securityLabel")]
    pub security_label: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub query: Option<Base64Binary>,
    pub detail: Option<Vec<AuditEventEntityDetail>>,
    pub agent: Option<Vec<AuditEventAgent>>,
}

/// Choice of types for the value\[x\] field in AuditEventEntityDetail
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum AuditEventEntityDetailValue {
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
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Base64Binary type.
    #[fhir_serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct AuditEventEntityDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<AuditEventEntityDetailValue>,
}

/// Choice of types for the occurred\[x\] field in AuditEvent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurred")]
pub enum AuditEventOccurred {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurredPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurredDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "occurred")]
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
    pub r#type: CodeableConcept,
    pub subtype: Option<Vec<CodeableConcept>>,
    pub action: Option<Code>,
    pub severity: Option<Code>,
    #[fhir_serde(flatten)]
    pub occurred: Option<AuditEventOccurred>,
    pub recorded: Instant,
    pub outcome: Option<AuditEventOutcome>,
    pub authorization: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub patient: Option<Reference>,
    pub encounter: Option<Reference>,
    pub agent: Option<Vec<AuditEventAgent>>,
    pub source: AuditEventSource,
    pub entity: Option<Vec<AuditEventEntity>>,
}

/// Choice of types for the network\[x\] field in AuditEventAgent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "network")]
pub enum AuditEventAgentNetwork {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "networkReference")]
    Reference(Reference),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "networkUri")]
    Uri(Uri),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "networkString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "network")]
pub struct AuditEventAgent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Reference,
    pub requestor: Option<Boolean>,
    pub location: Option<Reference>,
    pub policy: Option<Vec<Uri>>,
    #[fhir_serde(flatten)]
    pub network: Option<AuditEventAgentNetwork>,
    pub authorization: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AuditEventSource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub site: Option<Reference>,
    pub observer: Reference,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
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
    pub created: Option<DateTime>,
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

/// Choice of types for the value\[x\] field in BiologicallyDerivedProductProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum BiologicallyDerivedProductPropertyValue {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
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
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct BiologicallyDerivedProductProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<BiologicallyDerivedProductPropertyValue>,
}

/// Choice of types for the collected\[x\] field in BiologicallyDerivedProductCollection
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "collected")]
pub enum BiologicallyDerivedProductCollectionCollected {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "collectedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "collected")]
pub struct BiologicallyDerivedProductCollection {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub collector: Option<Reference>,
    pub source: Option<Reference>,
    #[fhir_serde(flatten)]
    pub collected: Option<BiologicallyDerivedProductCollectionCollected>,
    pub procedure: Option<Reference>,
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
    #[fhir_serde(rename = "productCategory")]
    pub product_category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "productCode")]
    pub product_code: Option<CodeableConcept>,
    pub parent: Option<Vec<Reference>>,
    pub request: Option<Vec<Reference>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "biologicalSourceEvent")]
    pub biological_source_event: Option<Identifier>,
    #[fhir_serde(rename = "processingFacility")]
    pub processing_facility: Option<Vec<Reference>>,
    pub division: Option<String>,
    #[fhir_serde(rename = "productStatus")]
    pub product_status: Option<Coding>,
    #[fhir_serde(rename = "expirationDate")]
    pub expiration_date: Option<DateTime>,
    pub collection: Option<BiologicallyDerivedProductCollection>,
    #[fhir_serde(rename = "storageTempRequirements")]
    pub storage_temp_requirements: Option<Range>,
    pub property: Option<Vec<BiologicallyDerivedProductProperty>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BiologicallyDerivedProductDispensePerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BiologicallyDerivedProductDispense {
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
    #[fhir_serde(rename = "originRelationshipType")]
    pub origin_relationship_type: Option<CodeableConcept>,
    pub product: Reference,
    pub subject: Reference,
    #[fhir_serde(rename = "matchStatus")]
    pub match_status: Option<CodeableConcept>,
    pub performer: Option<Vec<BiologicallyDerivedProductDispensePerformer>>,
    pub location: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "preparedDate")]
    pub prepared_date: Option<DateTime>,
    #[fhir_serde(rename = "whenHandedOver")]
    pub when_handed_over: Option<DateTime>,
    pub destination: Option<Reference>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "usageInstruction")]
    pub usage_instruction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BodyStructureIncludedStructureBodyLandmarkOrientation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "landmarkDescription")]
    pub landmark_description: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "clockFacePosition")]
    pub clock_face_position: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "distanceFromLandmark")]
    pub distance_from_landmark:
        Option<Vec<BodyStructureIncludedStructureBodyLandmarkOrientationDistanceFromLandmark>>,
    #[fhir_serde(rename = "surfaceOrientation")]
    pub surface_orientation: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BodyStructureIncludedStructureBodyLandmarkOrientationDistanceFromLandmark {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub device: Option<Vec<CodeableReference>>,
    pub value: Option<Vec<Quantity>>,
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
    #[fhir_serde(rename = "includedStructure")]
    pub included_structure: Option<Vec<BodyStructureIncludedStructure>>,
    #[fhir_serde(rename = "excludedStructure")]
    pub excluded_structure: Option<Vec<BodyStructureIncludedStructure>>,
    pub description: Option<Markdown>,
    pub image: Option<Vec<Attachment>>,
    pub patient: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BodyStructureIncludedStructure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub structure: CodeableConcept,
    pub laterality: Option<CodeableConcept>,
    #[fhir_serde(rename = "bodyLandmarkOrientation")]
    pub body_landmark_orientation:
        Option<Vec<BodyStructureIncludedStructureBodyLandmarkOrientation>>,
    #[fhir_serde(rename = "spatialReference")]
    pub spatial_reference: Option<Vec<Reference>>,
    pub qualifier: Option<Vec<CodeableConcept>>,
    pub morphology: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct BundleLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: Code,
    pub url: Uri,
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
    pub resource: Option<Box<Resource>>,
    pub search: Option<BundleEntrySearch>,
    pub request: Option<BundleEntryRequest>,
    pub response: Option<BundleEntryResponse>,
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
    pub issues: Option<Box<Resource>>,
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
    pub outcome: Option<Box<Resource>>,
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
pub struct CapabilityStatementDocument {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub documentation: Option<Markdown>,
    pub profile: Canonical,
}

/// Choice of types for the versionAlgorithm\[x\] field in CapabilityStatement
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum CapabilityStatementVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<CapabilityStatementVersionAlgorithm>,
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
    #[fhir_serde(rename = "actorDefinition")]
    pub actor_definition: Option<Vec<Canonical>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
    #[fhir_serde(rename = "acceptLanguage")]
    pub accept_language: Option<Vec<Code>>,
    #[fhir_serde(rename = "implementationGuide")]
    pub implementation_guide: Option<Vec<Canonical>>,
    pub rest: Option<Vec<CapabilityStatementRest>>,
    pub messaging: Option<Vec<CapabilityStatementMessaging>>,
    pub document: Option<Vec<CapabilityStatementDocument>>,
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
pub struct CapabilityStatementRestResource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub definition: Option<Canonical>,
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
    #[fhir_serde(rename = "conditionalPatch")]
    pub conditional_patch: Option<Boolean>,
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
pub struct CapabilityStatementRestInteraction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub documentation: Option<Markdown>,
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
pub struct CapabilityStatementImplementation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Markdown,
    pub url: Option<Url>,
    pub custodian: Option<Reference>,
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
pub struct CapabilityStatementMessagingEndpoint {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub protocol: Coding,
    pub address: Url,
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
    pub custodian: Option<Reference>,
    pub contributor: Option<Vec<Reference>>,
    #[fhir_serde(rename = "careTeam")]
    pub care_team: Option<Vec<Reference>>,
    pub addresses: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub goal: Option<Vec<Reference>>,
    pub activity: Option<Vec<CarePlanActivity>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CarePlanActivity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "performedActivity")]
    pub performed_activity: Option<Vec<CodeableReference>>,
    pub progress: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "plannedActivityReference")]
    pub planned_activity_reference: Option<Reference>,
}

/// Choice of types for the effective\[x\] field in CareTeamParticipant
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "effective")]
pub enum CareTeamParticipantEffective {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "effectiveTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "effective")]
pub struct CareTeamParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub member: Option<Reference>,
    #[fhir_serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    #[fhir_serde(flatten)]
    pub effective: Option<CareTeamParticipantEffective>,
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
    pub period: Option<Period>,
    pub participant: Option<Vec<CareTeamParticipant>>,
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub note: Option<Vec<Annotation>>,
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

/// Choice of types for the occurrence\[x\] field in ChargeItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "occurrence")]
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
    pub encounter: Option<Reference>,
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
    #[fhir_serde(rename = "unitPriceComponent")]
    pub unit_price_component: Option<MonetaryComponent>,
    #[fhir_serde(rename = "totalPriceComponent")]
    pub total_price_component: Option<MonetaryComponent>,
    #[fhir_serde(rename = "overrideReason")]
    pub override_reason: Option<CodeableConcept>,
    pub enterer: Option<Reference>,
    #[fhir_serde(rename = "enteredDate")]
    pub entered_date: Option<DateTime>,
    pub reason: Option<Vec<CodeableReference>>,
    pub service: Option<Vec<CodeableReference>>,
    pub product: Option<Vec<CodeableReference>>,
    pub account: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in ChargeItemDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ChargeItemDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<ChargeItemDefinitionVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "derivedFromUri")]
    pub derived_from_uri: Option<Vec<Uri>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Canonical>>,
    pub replaces: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub account: Option<Vec<Reference>>,
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
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    pub code: Option<CodeableConcept>,
    pub instance: Option<Vec<Reference>>,
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[fhir_serde(rename = "propertyGroup")]
    pub property_group: Option<Vec<ChargeItemDefinitionPropertyGroup>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ChargeItemDefinitionApplicability {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Option<Expression>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<RelatedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ChargeItemDefinitionPropertyGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[fhir_serde(rename = "priceComponent")]
    pub price_component: Option<Vec<MonetaryComponent>>,
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
pub struct CitationCitedArtifactContributorshipSummary {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub style: Option<Vec<CodeableConcept>>,
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
    #[fhir_serde(rename = "citedMedium")]
    pub cited_medium: Option<CodeableConcept>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    #[fhir_serde(rename = "articleDate")]
    pub article_date: Option<DateTime>,
    #[fhir_serde(rename = "publicationDateText")]
    pub publication_date_text: Option<String>,
    #[fhir_serde(rename = "publicationDateSeason")]
    pub publication_date_season: Option<String>,
    #[fhir_serde(rename = "lastRevisionDate")]
    pub last_revision_date: Option<DateTime>,
    pub language: Option<Vec<Code>>,
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

/// Choice of types for the target\[x\] field in CitationRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "target")]
pub enum CitationRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "targetMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "target")]
pub struct CitationRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
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
pub struct CitationCitedArtifactWebLocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub classifier: Option<Vec<CodeableConcept>>,
    pub url: Option<Uri>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactContributorshipEntry {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub contributor: Reference,
    #[fhir_serde(rename = "forenameInitials")]
    pub forename_initials: Option<String>,
    pub affiliation: Option<Vec<Reference>>,
    #[fhir_serde(rename = "contributionType")]
    pub contribution_type: Option<Vec<CodeableConcept>>,
    pub role: Option<CodeableConcept>,
    #[fhir_serde(rename = "contributionInstance")]
    pub contribution_instance:
        Option<Vec<CitationCitedArtifactContributorshipEntryContributionInstance>>,
    #[fhir_serde(rename = "correspondingContact")]
    pub corresponding_contact: Option<Boolean>,
    #[fhir_serde(rename = "rankingOrder")]
    pub ranking_order: Option<PositiveInt>,
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
    pub version: Option<String>,
    #[fhir_serde(rename = "currentState")]
    pub current_state: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<Vec<CitationCitedArtifactStatusDate>>,
    pub title: Option<Vec<CitationCitedArtifactTitle>>,
    #[fhir_serde(rename = "abstract")]
    pub r#abstract: Option<Vec<CitationCitedArtifactAbstract>>,
    pub part: Option<CitationCitedArtifactPart>,
    #[fhir_serde(rename = "baseCitation")]
    pub base_citation: Option<Reference>,
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
pub struct CitationCitedArtifactClassification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub classifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "artifactAssessment")]
    pub artifact_assessment: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactAbstract {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub language: Option<Code>,
    pub text: Markdown,
    pub copyright: Option<Markdown>,
}

/// Choice of types for the versionAlgorithm\[x\] field in Citation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum CitationVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<CitationVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub author: Option<Vec<ContactDetail>>,
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
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub recorder: Option<Vec<ContactDetail>>,
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

/// Choice of types for the target\[x\] field in CitationCitedArtifactRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "target")]
pub enum CitationCitedArtifactRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "target")]
pub struct CitationCitedArtifactRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub classifier: Option<Vec<CodeableConcept>>,
    pub label: Option<String>,
    pub display: Option<String>,
    pub citation: Option<Markdown>,
    #[fhir_serde(flatten)]
    pub target: Option<CitationCitedArtifactRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CitationCitedArtifactTitle {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub language: Option<Code>,
    pub text: Markdown,
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
#[fhir_choice_element(base_name = "diagnosis")]
pub enum ClaimDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "diagnosis")]
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
}

/// Choice of types for the serviced\[x\] field in ClaimItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "serviced")]
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
#[fhir_choice_element(base_name = "location")]
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
#[fhir_resource(choice_elements = "serviced,location")]
pub struct ClaimItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
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
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub request: Option<Vec<Reference>>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub serviced: Option<ClaimItemServiced>,
    #[fhir_serde(flatten)]
    pub location: Option<ClaimItemLocation>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<ClaimItemBodySite>>,
    pub encounter: Option<Vec<Reference>>,
    pub detail: Option<Vec<ClaimItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimItemBodySite {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub site: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
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

/// Choice of types for the timing\[x\] field in ClaimSupportingInfo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "timing")]
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
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "valueIdentifier")]
    Identifier(Identifier),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "timing,value")]
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

/// Choice of types for the when\[x\] field in ClaimEvent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "when")]
pub enum ClaimEventWhen {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "whenDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "whenPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "when")]
pub struct ClaimEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub when: Option<ClaimEventWhen>,
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
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
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
    pub provider: Option<Reference>,
    pub priority: Option<CodeableConcept>,
    #[fhir_serde(rename = "fundsReserve")]
    pub funds_reserve: Option<CodeableConcept>,
    pub related: Option<Vec<ClaimRelated>>,
    pub prescription: Option<Reference>,
    #[fhir_serde(rename = "originalPrescription")]
    pub original_prescription: Option<Reference>,
    pub payee: Option<ClaimPayee>,
    pub referral: Option<Reference>,
    pub encounter: Option<Vec<Reference>>,
    pub facility: Option<Reference>,
    #[fhir_serde(rename = "diagnosisRelatedGroup")]
    pub diagnosis_related_group: Option<CodeableConcept>,
    pub event: Option<Vec<ClaimEvent>>,
    #[fhir_serde(rename = "careTeam")]
    pub care_team: Option<Vec<ClaimCareTeam>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<ClaimSupportingInfo>>,
    pub diagnosis: Option<Vec<ClaimDiagnosis>>,
    pub procedure: Option<Vec<ClaimProcedure>>,
    pub insurance: Option<Vec<ClaimInsurance>>,
    pub accident: Option<ClaimAccident>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub item: Option<Vec<ClaimItem>>,
    pub total: Option<Money>,
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

/// Choice of types for the location\[x\] field in ClaimAccident
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "location")]
pub enum ClaimAccidentLocation {
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "location")]
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

/// Choice of types for the procedure\[x\] field in ClaimProcedure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "procedure")]
pub enum ClaimProcedureProcedure {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "procedureReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "procedure")]
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
pub struct ClaimItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ClaimItemDetailSubDetail>>,
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
    pub specialty: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseAddItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ClaimResponseItemReviewOutcome>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
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
pub struct ClaimResponseAddItemBodySite {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub site: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseProcessNote {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<PositiveInt>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub text: String,
    pub language: Option<CodeableConcept>,
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
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    pub created: DateTime,
    pub insurer: Option<Reference>,
    pub requestor: Option<Reference>,
    pub request: Option<Reference>,
    pub outcome: Code,
    pub decision: Option<CodeableConcept>,
    pub disposition: Option<String>,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<String>,
    #[fhir_serde(rename = "preAuthPeriod")]
    pub pre_auth_period: Option<Period>,
    pub event: Option<Vec<ClaimResponseEvent>>,
    #[fhir_serde(rename = "payeeType")]
    pub payee_type: Option<CodeableConcept>,
    pub encounter: Option<Vec<Reference>>,
    #[fhir_serde(rename = "diagnosisRelatedGroup")]
    pub diagnosis_related_group: Option<CodeableConcept>,
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
pub struct ClaimResponseItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "itemSequence")]
    pub item_sequence: PositiveInt,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ClaimResponseItemReviewOutcome>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub detail: Option<Vec<ClaimResponseItemDetail>>,
}

/// Choice of types for the serviced\[x\] field in ClaimResponseAddItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "serviced")]
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
#[fhir_choice_element(base_name = "location")]
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
#[fhir_resource(choice_elements = "serviced,location")]
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
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub provider: Option<Vec<Reference>>,
    pub revenue: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub request: Option<Vec<Reference>>,
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
    pub tax: Option<Money>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<ClaimResponseAddItemBodySite>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ClaimResponseItemReviewOutcome>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub detail: Option<Vec<ClaimResponseAddItemDetail>>,
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
    pub quantity: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "detailSequence")]
    pub detail_sequence: PositiveInt,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ClaimResponseItemReviewOutcome>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ClaimResponseItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseItemReviewOutcome {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub decision: Option<CodeableConcept>,
    pub reason: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<String>,
    #[fhir_serde(rename = "preAuthPeriod")]
    pub pre_auth_period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseAddItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ClaimResponseItemReviewOutcome>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClaimResponseItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: PositiveInt,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ClaimResponseItemReviewOutcome>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
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
    pub expression: Option<Vec<String>>,
}

/// Choice of types for the when\[x\] field in ClaimResponseEvent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "when")]
pub enum ClaimResponseEventWhen {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "whenDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "whenPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "when")]
pub struct ClaimResponseEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub when: Option<ClaimResponseEventWhen>,
}

/// Choice of types for the effective\[x\] field in ClinicalAssessment
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "effective")]
pub enum ClinicalAssessmentEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "effective")]
pub struct ClinicalAssessment {
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
    pub description: Option<String>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub effective: Option<ClinicalAssessmentEffective>,
    pub date: Option<DateTime>,
    pub performer: Option<Reference>,
    pub previous: Option<Reference>,
    pub problem: Option<Vec<Reference>>,
    #[fhir_serde(rename = "changePattern")]
    pub change_pattern: Option<CodeableConcept>,
    pub protocol: Option<Vec<Uri>>,
    pub summary: Option<Markdown>,
    pub finding: Option<Vec<ClinicalAssessmentFinding>>,
    #[fhir_serde(rename = "prognosisCodeableConcept")]
    pub prognosis_codeable_concept: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "prognosisReference")]
    pub prognosis_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalAssessmentFinding {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: Option<CodeableReference>,
    pub basis: Option<String>,
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
    pub applicability: Option<Expression>,
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
    pub subject: Option<Vec<CodeableReference>>,
    pub status: Option<CodeableConcept>,
    pub contraindication: Option<ClinicalUseDefinitionContraindication>,
    pub indication: Option<ClinicalUseDefinitionIndication>,
    pub interaction: Option<ClinicalUseDefinitionInteraction>,
    pub population: Option<Vec<Reference>>,
    pub library: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "undesirableEffect")]
    pub undesirable_effect: Option<ClinicalUseDefinitionUndesirableEffect>,
    pub warning: Option<ClinicalUseDefinitionWarning>,
}

/// Choice of types for the item\[x\] field in ClinicalUseDefinitionInteractionInteractant
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "item")]
pub enum ClinicalUseDefinitionInteractionInteractantItem {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "itemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "item")]
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

/// Choice of types for the duration\[x\] field in ClinicalUseDefinitionIndication
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "duration")]
pub enum ClinicalUseDefinitionIndicationDuration {
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "durationRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "durationString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "duration")]
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
    pub intended_effect: Option<Vec<CodeableReference>>,
    #[fhir_serde(flatten)]
    pub duration: Option<ClinicalUseDefinitionIndicationDuration>,
    #[fhir_serde(rename = "undesirableEffect")]
    pub undesirable_effect: Option<Vec<Reference>>,
    pub applicability: Option<Expression>,
    #[fhir_serde(rename = "otherTherapy")]
    pub other_therapy: Option<Vec<ClinicalUseDefinitionContraindicationOtherTherapy>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ClinicalUseDefinitionContraindicationOtherTherapy {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "relationshipType")]
    pub relationship_type: CodeableConcept,
    pub treatment: CodeableReference,
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

/// Choice of types for the versionAlgorithm\[x\] field in CodeSystem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum CodeSystemVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<CodeSystemVersionAlgorithm>,
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

/// Choice of types for the value\[x\] field in CodeSystemConceptProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
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
pub struct CodeSystemConceptDesignation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<Code>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Coding>,
    #[fhir_serde(rename = "additionalUse")]
    pub additional_use: Option<Vec<Coding>>,
    pub value: String,
}

/// Choice of types for the content\[x\] field in CommunicationPayload
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "content")]
pub enum CommunicationPayloadContent {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "contentCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "content")]
pub struct CommunicationPayload {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<CommunicationPayloadContent>,
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
    pub reason: Option<Vec<CodeableReference>>,
    pub payload: Option<Vec<CommunicationPayload>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the content\[x\] field in CommunicationRequestPayload
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "content")]
pub enum CommunicationRequestPayloadContent {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "contentCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "content")]
pub struct CommunicationRequestPayload {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<CommunicationRequestPayloadContent>,
}

/// Choice of types for the occurrence\[x\] field in CommunicationRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
pub enum CommunicationRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "occurrence")]
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
    pub intent: Code,
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
    #[fhir_serde(rename = "informationProvider")]
    pub information_provider: Option<Vec<Reference>>,
    pub reason: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
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
    #[fhir_serde(rename = "startParam")]
    pub start_param: Option<Uri>,
    #[fhir_serde(rename = "endParam")]
    pub end_param: Option<Uri>,
}

/// Choice of types for the versionAlgorithm\[x\] field in CompartmentDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum CompartmentDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<CompartmentDefinitionVersionAlgorithm>,
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
    pub purpose: Option<Markdown>,
    pub code: Code,
    pub search: Boolean,
    pub resource: Option<Vec<CompartmentDefinitionResource>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CompositionEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub period: Option<Period>,
    pub detail: Option<Vec<CodeableReference>>,
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
    #[fhir_serde(rename = "orderedBy")]
    pub ordered_by: Option<CodeableConcept>,
    pub entry: Option<Vec<Reference>>,
    #[fhir_serde(rename = "emptyReason")]
    pub empty_reason: Option<CodeableConcept>,
    pub section: Option<Vec<CompositionSection>>,
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
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    pub date: DateTime,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub author: Option<Vec<Reference>>,
    pub name: Option<String>,
    pub title: String,
    pub note: Option<Vec<Annotation>>,
    pub attester: Option<Vec<CompositionAttester>>,
    pub custodian: Option<Reference>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<CompositionRelatesTo>>,
    pub event: Option<Vec<CompositionEvent>>,
    pub section: Option<Vec<CompositionSection>>,
}

/// Choice of types for the target\[x\] field in CompositionRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "target")]
pub enum CompositionRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "targetMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "target")]
pub struct CompositionRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(flatten)]
    pub target: Option<CompositionRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CompositionAttester {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: CodeableConcept,
    pub time: Option<DateTime>,
    pub party: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMapGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: Option<Canonical>,
    pub target: Option<Canonical>,
    pub element: Option<Vec<ConceptMapGroupElement>>,
    pub unmapped: Option<ConceptMapGroupUnmapped>,
}

/// Choice of types for the versionAlgorithm\[x\] field in ConceptMap
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ConceptMapVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

/// Choice of types for the sourceScope\[x\] field in ConceptMap
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "sourceScope")]
pub enum ConceptMapSourceScope {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "sourceScopeUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "sourceScopeCanonical")]
    Canonical(Canonical),
}

/// Choice of types for the targetScope\[x\] field in ConceptMap
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "targetScope")]
pub enum ConceptMapTargetScope {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetScopeUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetScopeCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm,sourceScope,targetScope")]
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
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<ConceptMapVersionAlgorithm>,
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
    pub property: Option<Vec<ConceptMapProperty>>,
    #[fhir_serde(rename = "additionalAttribute")]
    pub additional_attribute: Option<Vec<ConceptMapAdditionalAttribute>>,
    #[fhir_serde(flatten)]
    pub source_scope: Option<ConceptMapSourceScope>,
    #[fhir_serde(flatten)]
    pub target_scope: Option<ConceptMapTargetScope>,
    pub group: Option<Vec<ConceptMapGroup>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMapAdditionalAttribute {
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
pub struct ConceptMapProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub uri: Option<Uri>,
    pub description: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub system: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConceptMapGroupElementTarget {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Code>,
    pub display: Option<String>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub relationship: Code,
    pub comment: Option<String>,
    pub property: Option<Vec<ConceptMapGroupElementTargetProperty>>,
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
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    #[fhir_serde(rename = "noMap")]
    pub no_map: Option<Boolean>,
    pub target: Option<Vec<ConceptMapGroupElementTarget>>,
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
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub relationship: Option<Code>,
    #[fhir_serde(rename = "otherMap")]
    pub other_map: Option<Canonical>,
}

/// Choice of types for the value\[x\] field in ConceptMapGroupElementTargetDependsOn
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum ConceptMapGroupElementTargetDependsOnValue {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct ConceptMapGroupElementTargetDependsOn {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub attribute: Code,
    #[fhir_serde(flatten)]
    pub value: Option<ConceptMapGroupElementTargetDependsOnValue>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
}

/// Choice of types for the value\[x\] field in ConceptMapGroupElementTargetProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum ConceptMapGroupElementTargetPropertyValue {
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
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "valueCode")]
    Code(Code),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct ConceptMapGroupElementTargetProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[fhir_serde(flatten)]
    pub value: Option<ConceptMapGroupElementTargetPropertyValue>,
}

/// Choice of types for the onset\[x\] field in Condition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "onset")]
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
#[fhir_choice_element(base_name = "abatement")]
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
#[fhir_resource(choice_elements = "onset,abatement")]
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
    pub clinical_status: CodeableConcept,
    #[fhir_serde(rename = "verificationStatus")]
    pub verification_status: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub severity: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "bodyStructure")]
    pub body_structure: Option<Reference>,
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
    pub evidence: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
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

/// Choice of types for the value\[x\] field in ConditionDefinitionPrecondition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum ConditionDefinitionPreconditionValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct ConditionDefinitionPrecondition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub code: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<ConditionDefinitionPreconditionValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConditionDefinitionQuestionnaire {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub purpose: Code,
    pub reference: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConditionDefinitionMedication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConditionDefinitionPlan {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub reference: Reference,
}

/// Choice of types for the versionAlgorithm\[x\] field in ConditionDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ConditionDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
pub struct ConditionDefinition {
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
    pub version_algorithm: Option<ConditionDefinitionVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub severity: Option<CodeableConcept>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    pub stage: Option<CodeableConcept>,
    #[fhir_serde(rename = "hasSeverity")]
    pub has_severity: Option<Boolean>,
    #[fhir_serde(rename = "hasBodySite")]
    pub has_body_site: Option<Boolean>,
    #[fhir_serde(rename = "hasStage")]
    pub has_stage: Option<Boolean>,
    pub definition: Option<Vec<Uri>>,
    pub observation: Option<Vec<Canonical>>,
    pub medication: Option<Vec<ConditionDefinitionMedication>>,
    pub precondition: Option<Vec<ConditionDefinitionPrecondition>>,
    pub team: Option<Vec<Reference>>,
    pub questionnaire: Option<Vec<ConditionDefinitionQuestionnaire>>,
    pub plan: Option<Vec<ConditionDefinitionPlan>>,
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
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub date: Option<Date>,
    pub period: Option<Period>,
    pub grantor: Option<Vec<Reference>>,
    pub grantee: Option<Vec<Reference>>,
    pub manager: Option<Vec<Reference>>,
    pub controller: Option<Vec<Reference>>,
    #[fhir_serde(rename = "sourceAttachment")]
    pub source_attachment: Option<Vec<Attachment>>,
    #[fhir_serde(rename = "sourceReference")]
    pub source_reference: Option<Vec<Reference>>,
    #[fhir_serde(rename = "regulatoryBasis")]
    pub regulatory_basis: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "policyBasis")]
    pub policy_basis: Option<ConsentPolicyBasis>,
    #[fhir_serde(rename = "policyText")]
    pub policy_text: Option<Vec<Reference>>,
    pub verification: Option<Vec<ConsentVerification>>,
    pub decision: Option<Code>,
    #[fhir_serde(rename = "provisionReference")]
    pub provision_reference: Option<Vec<Reference>>,
    pub provision: Option<Vec<ConsentProvision>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConsentProvision {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub period: Option<Period>,
    pub actor: Option<Vec<ConsentProvisionActor>>,
    pub action: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "securityLabel")]
    pub security_label: Option<Vec<Coding>>,
    pub purpose: Option<Vec<Coding>>,
    #[fhir_serde(rename = "documentType")]
    pub document_type: Option<Vec<Coding>>,
    #[fhir_serde(rename = "resourceType")]
    pub resource_type: Option<Vec<Coding>>,
    pub code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "dataPeriod")]
    pub data_period: Option<Period>,
    pub data: Option<Vec<ConsentProvisionData>>,
    pub expression: Option<Expression>,
    pub provision: Option<Vec<ConsentProvision>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConsentVerification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub verified: Boolean,
    #[fhir_serde(rename = "verificationType")]
    pub verification_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "verifiedBy")]
    pub verified_by: Option<Reference>,
    #[fhir_serde(rename = "verifiedWith")]
    pub verified_with: Option<Reference>,
    #[fhir_serde(rename = "verificationDate")]
    pub verification_date: Option<Vec<DateTime>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ConsentProvisionActor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub reference: Option<Reference>,
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
pub struct ConsentPolicyBasis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Reference>,
    pub uri: Option<Uri>,
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

/// Choice of types for the occurrence\[x\] field in ContractTermAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
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
#[fhir_resource(choice_elements = "occurrence")]
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
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "reasonLinkId")]
    pub reason_link_id: Option<Vec<String>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
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
pub struct ContractTermOfferParty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Vec<Reference>>,
    pub role: CodeableConcept,
}

/// Choice of types for the entity\[x\] field in ContractTermAssetValuedItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "entity")]
pub enum ContractTermAssetValuedItemEntity {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "entityCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "entityReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "entity")]
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

/// Choice of types for the content\[x\] field in ContractLegal
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "content")]
pub enum ContractLegalContent {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "content")]
pub struct ContractLegal {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<ContractLegalContent>,
}

/// Choice of types for the content\[x\] field in ContractRule
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "content")]
pub enum ContractRuleContent {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "content")]
pub struct ContractRule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<ContractRuleContent>,
}

/// Choice of types for the topic\[x\] field in ContractTerm
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "topic")]
pub enum ContractTermTopic {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "topicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "topicReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "topic")]
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

/// Choice of types for the value\[x\] field in ContractTermOfferAnswer
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
pub struct ContractTermOfferAnswer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<ContractTermOfferAnswerValue>,
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

/// Choice of types for the topic\[x\] field in Contract
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "topic")]
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
#[fhir_choice_element(base_name = "legallyBinding")]
pub enum ContractLegallyBinding {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "legallyBindingAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "legallyBindingReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "topic,legallyBinding")]
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

/// Choice of types for the content\[x\] field in ContractFriendly
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "content")]
pub enum ContractFriendlyContent {
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "content")]
pub struct ContractFriendly {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub content: Option<ContractFriendlyContent>,
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
    pub kind: Code,
    #[fhir_serde(rename = "paymentBy")]
    pub payment_by: Option<Vec<CoveragePaymentBy>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "policyHolder")]
    pub policy_holder: Option<Reference>,
    pub subscriber: Option<Reference>,
    #[fhir_serde(rename = "subscriberId")]
    pub subscriber_id: Option<Vec<Identifier>>,
    pub beneficiary: Reference,
    pub dependent: Option<String>,
    pub relationship: Option<CodeableConcept>,
    pub period: Option<Period>,
    pub insurer: Option<Reference>,
    pub class: Option<Vec<CoverageClass>>,
    pub order: Option<PositiveInt>,
    pub network: Option<String>,
    #[fhir_serde(rename = "costToBeneficiary")]
    pub cost_to_beneficiary: Option<Vec<CoverageCostToBeneficiary>>,
    pub subrogation: Option<Boolean>,
    pub contract: Option<Vec<Reference>>,
    #[fhir_serde(rename = "insurancePlan")]
    pub insurance_plan: Option<Reference>,
}

/// Choice of types for the value\[x\] field in CoverageCostToBeneficiary
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum CoverageCostToBeneficiaryValue {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "valueMoney")]
    Money(Money),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct CoverageCostToBeneficiary {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub network: Option<CodeableConcept>,
    pub unit: Option<CodeableConcept>,
    pub term: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<CoverageCostToBeneficiaryValue>,
    pub exception: Option<Vec<CoverageCostToBeneficiaryException>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageClass {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub value: Identifier,
    pub name: Option<String>,
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
pub struct CoveragePaymentBy {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub party: Reference,
    pub responsibility: Option<String>,
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

/// Choice of types for the when\[x\] field in CoverageEligibilityRequestEvent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "when")]
pub enum CoverageEligibilityRequestEventWhen {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "whenDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "whenPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "when")]
pub struct CoverageEligibilityRequestEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub when: Option<CoverageEligibilityRequestEventWhen>,
}

/// Choice of types for the diagnosis\[x\] field in CoverageEligibilityRequestItemDiagnosis
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "diagnosis")]
pub enum CoverageEligibilityRequestItemDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "diagnosis")]
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
#[fhir_choice_element(base_name = "serviced")]
pub enum CoverageEligibilityRequestServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "serviced")]
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
    pub event: Option<Vec<CoverageEligibilityRequestEvent>>,
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

/// Choice of types for the when\[x\] field in CoverageEligibilityResponseEvent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "when")]
pub enum CoverageEligibilityResponseEventWhen {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "whenDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "whenPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "when")]
pub struct CoverageEligibilityResponseEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub when: Option<CoverageEligibilityResponseEventWhen>,
}

/// Choice of types for the serviced\[x\] field in CoverageEligibilityResponse
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "serviced")]
pub enum CoverageEligibilityResponseServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "serviced")]
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
    pub event: Option<Vec<CoverageEligibilityResponseEvent>>,
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

/// Choice of types for the allowed\[x\] field in CoverageEligibilityResponseInsuranceItemBenefit
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "allowed")]
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
#[fhir_choice_element(base_name = "used")]
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
#[fhir_resource(choice_elements = "allowed,used")]
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct CoverageEligibilityResponseError {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub expression: Option<Vec<String>>,
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

/// Choice of types for the identified\[x\] field in DetectedIssue
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "identified")]
pub enum DetectedIssueIdentified {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "identifiedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "identifiedPeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "identifiedTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "identified")]
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
    pub category: Option<Vec<CodeableConcept>>,
    pub code: Option<CodeableConcept>,
    pub severity: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub identified: Option<DetectedIssueIdentified>,
    pub author: Option<Reference>,
    pub implicated: Option<Vec<Reference>>,
    pub evidence: Option<Vec<DetectedIssueEvidence>>,
    pub detail: Option<Markdown>,
    pub reference: Option<Uri>,
    #[fhir_serde(rename = "qualityOfEvidence")]
    pub quality_of_evidence: Option<CodeableConcept>,
    #[fhir_serde(rename = "managementCode")]
    pub management_code: Option<CodeableConcept>,
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
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceAdditive {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableReference,
    pub quantity: Option<Quantity>,
    pub performer: Option<Reference>,
    pub performed: Option<DateTime>,
}

/// Choice of types for the value\[x\] field in DeviceProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum DevicePropertyValue {
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
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct DeviceProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<DevicePropertyValue>,
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
    #[fhir_serde(rename = "availabilityStatus")]
    pub availability_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "biologicalSourceEvent")]
    pub biological_source_event: Option<Identifier>,
    pub manufacturer: Option<String>,
    #[fhir_serde(rename = "manufactureDate")]
    pub manufacture_date: Option<DateTime>,
    #[fhir_serde(rename = "expirationDate")]
    pub expiration_date: Option<DateTime>,
    #[fhir_serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    #[fhir_serde(rename = "serialNumber")]
    pub serial_number: Option<String>,
    pub name: Option<Vec<DeviceName>>,
    #[fhir_serde(rename = "modelNumber")]
    pub model_number: Option<String>,
    #[fhir_serde(rename = "partNumber")]
    pub part_number: Option<String>,
    pub category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "deviceVersion")]
    pub device_version: Option<Vec<DeviceDeviceVersion>>,
    #[fhir_serde(rename = "conformsTo")]
    pub conforms_to: Option<Vec<DeviceConformsTo>>,
    pub property: Option<Vec<DeviceProperty>>,
    pub additive: Option<Vec<DeviceAdditive>>,
    pub contact: Option<Vec<ContactPoint>>,
    pub location: Option<Reference>,
    pub note: Option<Vec<Annotation>>,
    pub safety: Option<Vec<CodeableConcept>>,
    pub parent: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceConformsTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    pub specification: CodeableConcept,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDeviceVersion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub component: Option<Identifier>,
    #[fhir_serde(rename = "installDate")]
    pub install_date: Option<DateTime>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: String,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub display: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceUdiCarrier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "deviceIdentifier")]
    pub device_identifier: String,
    #[fhir_serde(rename = "deviceIdentifierSystem")]
    pub device_identifier_system: Option<Uri>,
    pub issuer: Uri,
    pub jurisdiction: Option<Uri>,
    #[fhir_serde(rename = "carrierAIDC")]
    pub carrier_a_i_d_c: Option<Base64Binary>,
    #[fhir_serde(rename = "carrierHRF")]
    pub carrier_h_r_f: Option<String>,
    #[fhir_serde(rename = "entryType")]
    pub entry_type: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceAlertDerivedFrom {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub observation: Reference,
    pub component: Option<Coding>,
    pub limit: Option<Range>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceAlertSignal {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "activationState")]
    pub activation_state: Code,
    pub presence: Option<Code>,
    pub annunciator: Option<CodeableReference>,
    pub manifestation: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub indication: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceAlert {
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
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub priority: Option<Code>,
    pub code: CodeableConcept,
    pub subject: Reference,
    pub presence: Boolean,
    #[fhir_serde(rename = "presencePeriod")]
    pub presence_period: Option<Period>,
    pub device: Option<Reference>,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<DeviceAlertDerivedFrom>>,
    pub acknowledged: Option<Boolean>,
    pub label: Option<String>,
    pub signal: Option<Vec<DeviceAlertSignal>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceAssociation {
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
    pub device: Reference,
    pub relationship: Option<Vec<CodeableConcept>>,
    pub status: CodeableConcept,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    #[fhir_serde(rename = "bodyStructure")]
    pub body_structure: Option<Reference>,
    pub period: Option<Period>,
    pub operation: Option<Vec<DeviceAssociationOperation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceAssociationOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: CodeableConcept,
    pub operator: Option<Vec<Reference>>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionChargeItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "chargeItemCode")]
    pub charge_item_code: CodeableReference,
    pub count: Quantity,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionPackagingDistributor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    #[fhir_serde(rename = "organizationReference")]
    pub organization_reference: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionCorrectiveAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub recall: Boolean,
    pub scope: Option<Code>,
    pub period: Period,
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
pub struct DeviceDefinitionPackaging {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub count: Option<Integer>,
    pub distributor: Option<Vec<DeviceDefinitionPackagingDistributor>>,
    #[fhir_serde(rename = "udiDeviceIdentifier")]
    pub udi_device_identifier: Option<Vec<DeviceDefinitionUdiDeviceIdentifier>>,
    pub packaging: Option<Vec<DeviceDefinitionPackaging>>,
}

/// Choice of types for the value\[x\] field in DeviceDefinitionProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum DeviceDefinitionPropertyValue {
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
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct DeviceDefinitionProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<DeviceDefinitionPropertyValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionConformsTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    pub specification: CodeableConcept,
    pub version: Option<Vec<String>>,
    pub source: Option<Vec<RelatedArtifact>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: Coding,
    #[fhir_serde(rename = "relatedDevice")]
    pub related_device: CodeableReference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionDeviceVersion {
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
pub struct DeviceDefinitionGuideline {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    #[fhir_serde(rename = "usageInstruction")]
    pub usage_instruction: Option<Markdown>,
    #[fhir_serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub indication: Option<Vec<CodeableConcept>>,
    pub contraindication: Option<Vec<CodeableConcept>>,
    pub warning: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "intendedUse")]
    pub intended_use: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionUdiDeviceIdentifierMarketDistribution {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "marketPeriod")]
    pub market_period: Period,
    #[fhir_serde(rename = "subJurisdiction")]
    pub sub_jurisdiction: Uri,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionDeviceName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionClassification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub justification: Option<Vec<RelatedArtifact>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionRegulatoryIdentifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(rename = "deviceIdentifier")]
    pub device_identifier: String,
    pub issuer: Uri,
    pub jurisdiction: Uri,
    #[fhir_serde(rename = "deviceIdentifierSystem")]
    pub device_identifier_system: Option<Uri>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDefinitionHasPart {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    pub count: Option<Integer>,
}

/// Choice of types for the versionAlgorithm\[x\] field in DeviceDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum DeviceDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<DeviceDefinitionVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(rename = "partNumber")]
    pub part_number: Option<String>,
    pub manufacturer: Option<Reference>,
    #[fhir_serde(rename = "modelNumber")]
    pub model_number: Option<String>,
    pub date: Option<DateTime>,
    pub contact: Option<Vec<ContactDetail>>,
    pub publisher: Option<String>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    #[fhir_serde(rename = "udiDeviceIdentifier")]
    pub udi_device_identifier: Option<Vec<DeviceDefinitionUdiDeviceIdentifier>>,
    #[fhir_serde(rename = "regulatoryIdentifier")]
    pub regulatory_identifier: Option<Vec<DeviceDefinitionRegulatoryIdentifier>>,
    #[fhir_serde(rename = "deviceName")]
    pub device_name: Option<Vec<DeviceDefinitionDeviceName>>,
    pub classification: Option<Vec<DeviceDefinitionClassification>>,
    #[fhir_serde(rename = "conformsTo")]
    pub conforms_to: Option<Vec<DeviceDefinitionConformsTo>>,
    #[fhir_serde(rename = "hasPart")]
    pub has_part: Option<Vec<DeviceDefinitionHasPart>>,
    pub packaging: Option<Vec<DeviceDefinitionPackaging>>,
    #[fhir_serde(rename = "deviceVersion")]
    pub device_version: Option<Vec<DeviceDefinitionDeviceVersion>>,
    pub safety: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "shelfLifeStorage")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    #[fhir_serde(rename = "outputLanguage")]
    pub output_language: Option<Vec<Code>>,
    pub property: Option<Vec<DeviceDefinitionProperty>>,
    pub link: Option<Vec<DeviceDefinitionLink>>,
    pub note: Option<Vec<Annotation>>,
    pub material: Option<Vec<DeviceDefinitionMaterial>>,
    #[fhir_serde(rename = "productionIdentifierInUDI")]
    pub production_identifier_in_u_d_i: Option<Vec<CodeableConcept>>,
    pub guideline: Option<DeviceDefinitionGuideline>,
    #[fhir_serde(rename = "correctiveAction")]
    pub corrective_action: Option<DeviceDefinitionCorrectiveAction>,
    #[fhir_serde(rename = "chargeItem")]
    pub charge_item: Option<Vec<DeviceDefinitionChargeItem>>,
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
    #[fhir_serde(rename = "marketDistribution")]
    pub market_distribution: Option<Vec<DeviceDefinitionUdiDeviceIdentifierMarketDistribution>>,
    #[fhir_serde(rename = "deviceIdentifierSystem")]
    pub device_identifier_system: Option<Uri>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDispensePerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceDispense {
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
    pub status_reason: Option<CodeableReference>,
    pub category: Option<Vec<CodeableConcept>>,
    pub device: CodeableReference,
    pub subject: Reference,
    pub receiver: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    pub performer: Option<Vec<DeviceDispensePerformer>>,
    pub location: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "preparedDate")]
    pub prepared_date: Option<DateTime>,
    #[fhir_serde(rename = "whenHandedOver")]
    pub when_handed_over: Option<DateTime>,
    pub destination: Option<Reference>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "usageInstruction")]
    pub usage_instruction: Option<Markdown>,
    #[fhir_serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceMetricCalibration {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub state: Option<Code>,
    pub time: Option<Instant>,
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
    pub device: Reference,
    #[fhir_serde(rename = "operationalStatus")]
    pub operational_status: Option<Code>,
    pub color: Option<Code>,
    pub category: CodeableConcept,
    #[fhir_serde(rename = "measurementFrequency")]
    pub measurement_frequency: Option<Quantity>,
    pub availability: Option<CodeableConcept>,
    pub calibration: Option<Vec<DeviceMetricCalibration>>,
}

/// Choice of types for the occurrence\[x\] field in DeviceRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
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
#[fhir_resource(choice_elements = "occurrence")]
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
    pub replaces: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Option<Code>,
    pub intent: Code,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    pub code: CodeableReference,
    pub quantity: Option<Integer>,
    pub parameter: Option<Vec<DeviceRequestParameter>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<DeviceRequestOccurrence>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub performer: Option<CodeableReference>,
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "asNeeded")]
    pub as_needed: Option<Boolean>,
    #[fhir_serde(rename = "asNeededFor")]
    pub as_needed_for: Option<CodeableConcept>,
    pub insurance: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
}

/// Choice of types for the value\[x\] field in DeviceRequestParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
pub struct DeviceRequestParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<DeviceRequestParameterValue>,
}

/// Choice of types for the timing\[x\] field in DeviceUsage
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "timing")]
pub enum DeviceUsageTiming {
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
#[fhir_resource(choice_elements = "timing")]
pub struct DeviceUsage {
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
    pub subject: Reference,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    pub context: Option<Reference>,
    #[fhir_serde(flatten)]
    pub timing: Option<DeviceUsageTiming>,
    #[fhir_serde(rename = "dateAsserted")]
    pub date_asserted: Option<DateTime>,
    #[fhir_serde(rename = "usageStatus")]
    pub usage_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "usageReason")]
    pub usage_reason: Option<Vec<CodeableConcept>>,
    pub adherence: Option<DeviceUsageAdherence>,
    #[fhir_serde(rename = "informationSource")]
    pub information_source: Option<Reference>,
    pub device: CodeableReference,
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableReference>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DeviceUsageAdherence {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub reason: Option<Vec<CodeableConcept>>,
}

/// Choice of types for the effective\[x\] field in DiagnosticReport
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "effective")]
pub enum DiagnosticReportEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "effective")]
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
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<RelatedArtifact>>,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub effective: Option<DiagnosticReportEffective>,
    pub issued: Option<Instant>,
    pub procedure: Option<Vec<Reference>>,
    pub performer: Option<Vec<Reference>>,
    #[fhir_serde(rename = "resultsInterpreter")]
    pub results_interpreter: Option<Vec<Reference>>,
    pub specimen: Option<Vec<Reference>>,
    pub result: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub study: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<DiagnosticReportSupportingInfo>>,
    pub media: Option<Vec<DiagnosticReportMedia>>,
    pub composition: Option<Reference>,
    pub conclusion: Option<Markdown>,
    #[fhir_serde(rename = "conclusionCode")]
    pub conclusion_code: Option<Vec<CodeableReference>>,
    pub recomendation: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "presentedForm")]
    pub presented_form: Option<Vec<Attachment>>,
    pub communication: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DiagnosticReportSupportingInfo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub reference: Reference,
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
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "docStatus")]
    pub doc_status: Option<Code>,
    pub modality: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub context: Option<Vec<Reference>>,
    pub event: Option<Vec<CodeableReference>>,
    pub related: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "facilityType")]
    pub facility_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "practiceSetting")]
    pub practice_setting: Option<CodeableConcept>,
    pub period: Option<Period>,
    pub date: Option<DateTime>,
    pub author: Option<Vec<Reference>>,
    pub attester: Option<Vec<DocumentReferenceAttester>>,
    pub custodian: Option<Reference>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<DocumentReferenceRelatesTo>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "securityLabel")]
    pub security_label: Option<Vec<CodeableConcept>>,
    pub content: Option<Vec<DocumentReferenceContent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentReferenceRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub target: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentReferenceContent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub attachment: Attachment,
    pub profile: Option<Vec<DocumentReferenceContentProfile>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct DocumentReferenceAttester {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: CodeableConcept,
    pub time: Option<DateTime>,
    pub party: Option<Reference>,
}

/// Choice of types for the value\[x\] field in DocumentReferenceContentProfile
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum DocumentReferenceContentProfileValue {
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "valueCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct DocumentReferenceContentProfile {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<DocumentReferenceContentProfileValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterDiagnosis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Vec<CodeableConcept>>,
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
    pub class: Option<Vec<CodeableConcept>>,
    pub priority: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableReference>>,
    pub subject: Option<Reference>,
    #[fhir_serde(rename = "subjectStatus")]
    pub subject_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "episodeOfCare")]
    pub episode_of_care: Option<Vec<Reference>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "careTeam")]
    pub care_team: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Reference>,
    #[fhir_serde(rename = "serviceProvider")]
    pub service_provider: Option<Reference>,
    pub participant: Option<Vec<EncounterParticipant>>,
    pub appointment: Option<Vec<Reference>>,
    #[fhir_serde(rename = "virtualService")]
    pub virtual_service: Option<Vec<VirtualServiceDetail>>,
    #[fhir_serde(rename = "actualPeriod")]
    pub actual_period: Option<Period>,
    #[fhir_serde(rename = "plannedStartDate")]
    pub planned_start_date: Option<DateTime>,
    #[fhir_serde(rename = "plannedEndDate")]
    pub planned_end_date: Option<DateTime>,
    pub length: Option<Duration>,
    pub reason: Option<Vec<EncounterReason>>,
    pub diagnosis: Option<Vec<EncounterDiagnosis>>,
    pub account: Option<Vec<Reference>>,
    #[fhir_serde(rename = "dietPreference")]
    pub diet_preference: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "specialArrangement")]
    pub special_arrangement: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "specialCourtesy")]
    pub special_courtesy: Option<Vec<CodeableConcept>>,
    pub admission: Option<EncounterAdmission>,
    pub location: Option<Vec<EncounterLocation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterReason {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Vec<CodeableConcept>>,
    pub value: Option<Vec<CodeableReference>>,
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
    pub actor: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterAdmission {
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
    pub destination: Option<Reference>,
    #[fhir_serde(rename = "dischargeDisposition")]
    pub discharge_disposition: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterLocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub location: Reference,
    pub status: Option<Code>,
    pub form: Option<CodeableConcept>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterHistoryLocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub location: Reference,
    pub form: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EncounterHistory {
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
    pub encounter: Option<Reference>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub class: CodeableConcept,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableReference>>,
    pub subject: Option<Reference>,
    #[fhir_serde(rename = "subjectStatus")]
    pub subject_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "actualPeriod")]
    pub actual_period: Option<Period>,
    #[fhir_serde(rename = "plannedStartDate")]
    pub planned_start_date: Option<DateTime>,
    #[fhir_serde(rename = "plannedEndDate")]
    pub planned_end_date: Option<DateTime>,
    pub length: Option<Duration>,
    pub location: Option<Vec<EncounterHistoryLocation>>,
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
    pub connection_type: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[fhir_serde(rename = "environmentType")]
    pub environment_type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub period: Option<Period>,
    pub availability: Option<Availability>,
    pub payload: Option<Vec<EndpointPayload>>,
    pub address: Url,
    pub header: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EndpointPayload {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "mimeType")]
    pub mime_type: Option<Vec<Code>>,
    #[fhir_serde(rename = "profileCanonical")]
    pub profile_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "profileUri")]
    pub profile_uri: Option<Vec<Uri>>,
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
pub struct EpisodeOfCareReason {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Vec<CodeableConcept>>,
    pub value: Option<Vec<CodeableReference>>,
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
    pub reason: Option<Vec<EpisodeOfCareReason>>,
    pub diagnosis: Option<Vec<EpisodeOfCareDiagnosis>>,
    pub subject: Reference,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub period: Option<Period>,
    #[fhir_serde(rename = "referralRequest")]
    pub referral_request: Option<Vec<Reference>>,
    #[fhir_serde(rename = "careManager")]
    pub care_manager: Option<Reference>,
    #[fhir_serde(rename = "careTeam")]
    pub care_team: Option<Vec<Reference>>,
    pub account: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EpisodeOfCareDiagnosis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Option<Vec<CodeableConcept>>,
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

/// Choice of types for the versionAlgorithm\[x\] field in EventDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum EventDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

/// Choice of types for the subject\[x\] field in EventDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
pub enum EventDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm,subject")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<EventDefinitionVersionAlgorithm>,
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
    pub usage: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
pub struct EvidenceStatistic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
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
pub struct EvidenceCertainty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub rating: Option<CodeableConcept>,
    pub rater: Option<Vec<String>>,
    pub subcomponent: Option<Vec<EvidenceCertainty>>,
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
    pub variable_role: Code,
    #[fhir_serde(rename = "roleSubtype")]
    pub role_subtype: Option<CodeableConcept>,
    #[fhir_serde(rename = "comparatorCategory")]
    pub comparator_category: Option<String>,
    pub observed: Option<Reference>,
    pub intended: Option<Reference>,
    #[fhir_serde(rename = "directnessMatch")]
    pub directness_match: Option<CodeableConcept>,
}

/// Choice of types for the versionAlgorithm\[x\] field in Evidence
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum EvidenceVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<EvidenceVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "citeAs")]
    pub cite_as: Option<Markdown>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    pub author: Option<Vec<ContactDetail>>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub recorder: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<EvidenceRelatesTo>>,
    pub description: Option<Markdown>,
    pub assertion: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "variableDefinition")]
    pub variable_definition: Option<Vec<EvidenceVariableDefinition>>,
    #[fhir_serde(rename = "synthesisType")]
    pub synthesis_type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "studyDesign")]
    pub study_design: Option<Vec<CodeableConcept>>,
    pub statistic: Option<Vec<EvidenceStatistic>>,
    pub certainty: Option<Vec<EvidenceCertainty>>,
}

/// Choice of types for the value\[x\] field in EvidenceStatisticModelCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum EvidenceStatisticModelCharacteristicValue {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct EvidenceStatisticModelCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<EvidenceStatisticModelCharacteristicValue>,
    pub intended: Option<Boolean>,
    pub applied: Option<Boolean>,
    pub variable: Option<Vec<EvidenceStatisticModelCharacteristicVariable>>,
    pub attribute: Option<Vec<EvidenceStatisticAttributeEstimate>>,
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

/// Choice of types for the target\[x\] field in EvidenceRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "target")]
pub enum EvidenceRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "targetMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "target")]
pub struct EvidenceRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(flatten)]
    pub target: Option<EvidenceRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceStatisticSampleSize {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "numberOfStudies")]
    pub number_of_studies: Option<UnsignedInt>,
    #[fhir_serde(rename = "numberOfParticipants")]
    pub number_of_participants: Option<UnsignedInt>,
    #[fhir_serde(rename = "knownDataCount")]
    pub known_data_count: Option<UnsignedInt>,
    #[fhir_serde(rename = "numberAnalyzed")]
    pub number_analyzed: Option<UnsignedInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceStatisticAttributeEstimate {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
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
pub struct EvidenceVariableConstraint {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub conditional: Option<CodeableConcept>,
    #[fhir_serde(rename = "minimumQuantity")]
    pub minimum_quantity: Option<Quantity>,
    #[fhir_serde(rename = "maximumQuantity")]
    pub maximum_quantity: Option<Quantity>,
    #[fhir_serde(rename = "earliestDateTime")]
    pub earliest_date_time: Option<DateTime>,
    #[fhir_serde(rename = "latestDateTime")]
    pub latest_date_time: Option<DateTime>,
    #[fhir_serde(rename = "minimumStringLength")]
    pub minimum_string_length: Option<UnsignedInt>,
    #[fhir_serde(rename = "maximumStringLength")]
    pub maximum_string_length: Option<PositiveInt>,
    pub code: Option<CodeableConcept>,
    pub expression: Option<Expression>,
    #[fhir_serde(rename = "expectedValueSet")]
    pub expected_value_set: Option<Reference>,
    #[fhir_serde(rename = "expectedUnitsValueSet")]
    pub expected_units_value_set: Option<Reference>,
    #[fhir_serde(rename = "anyValueAllowed")]
    pub any_value_allowed: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct EvidenceVariableDataStorage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub datatype: Option<CodeableConcept>,
    pub path: Option<String>,
    pub delimiter: Option<String>,
    pub component: Option<Vec<EvidenceVariableDataStorage>>,
}

/// Choice of types for the target\[x\] field in EvidenceVariableRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "target")]
pub enum EvidenceVariableRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "targetMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "target")]
pub struct EvidenceVariableRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(flatten)]
    pub target: Option<EvidenceVariableRelatesToTarget>,
}

/// Choice of types for the value\[x\] field in EvidenceVariableCategory
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct EvidenceVariableCategory {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    #[fhir_serde(flatten)]
    pub value: Option<EvidenceVariableCategoryValue>,
}

/// Choice of types for the value\[x\] field in EvidenceVariableDefinitionModifier
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum EvidenceVariableDefinitionModifierValue {
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
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the RelativeTime type.
    #[fhir_serde(rename = "valueRelativeTime")]
    RelativeTime(RelativeTime),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct EvidenceVariableDefinitionModifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<EvidenceVariableDefinitionModifierValue>,
}

/// Choice of types for the versionAlgorithm\[x\] field in EvidenceVariable
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum EvidenceVariableVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<EvidenceVariableVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "shortTitle")]
    pub short_title: Option<String>,
    #[fhir_serde(rename = "citeAs")]
    pub cite_as: Option<Markdown>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub author: Option<Vec<ContactDetail>>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub recorder: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<EvidenceVariableRelatesTo>>,
    pub actual: Option<Boolean>,
    pub definition: Option<CodeableReference>,
    #[fhir_serde(rename = "definitionModifier")]
    pub definition_modifier: Option<Vec<EvidenceVariableDefinitionModifier>>,
    pub handling: Option<CodeableConcept>,
    pub category: Option<Vec<EvidenceVariableCategory>>,
    pub conditional: Option<Expression>,
    pub classifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "dataStorage")]
    pub data_storage: Option<Vec<EvidenceVariableDataStorage>>,
    pub timing: Option<RelativeTime>,
    pub period: Option<Period>,
    pub constraint: Option<Vec<EvidenceVariableConstraint>>,
    #[fhir_serde(rename = "missingDataMeaning")]
    pub missing_data_meaning: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "unacceptableDataHandling")]
    pub unacceptable_data_handling: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioInstanceVersion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub key: String,
    pub title: String,
    pub description: Option<Markdown>,
    pub content: Option<Reference>,
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

/// Choice of types for the structureProfile\[x\] field in ExampleScenarioInstance
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "structureProfile")]
pub enum ExampleScenarioInstanceStructureProfile {
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "structureProfileCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "structureProfileUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "structureProfile")]
pub struct ExampleScenarioInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub key: String,
    #[fhir_serde(rename = "structureType")]
    pub structure_type: Coding,
    #[fhir_serde(rename = "structureVersion")]
    pub structure_version: Option<String>,
    #[fhir_serde(flatten)]
    pub structure_profile: Option<ExampleScenarioInstanceStructureProfile>,
    pub title: String,
    pub description: Option<Markdown>,
    pub content: Option<Reference>,
    pub version: Option<Vec<ExampleScenarioInstanceVersion>>,
    #[fhir_serde(rename = "containedInstance")]
    pub contained_instance: Option<Vec<ExampleScenarioInstanceContainedInstance>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioProcessStepOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Coding>,
    pub title: String,
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
pub struct ExampleScenarioInstanceContainedInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "instanceReference")]
    pub instance_reference: String,
    #[fhir_serde(rename = "versionReference")]
    pub version_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioProcessStep {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<String>,
    pub process: Option<ExampleScenarioProcess>,
    pub workflow: Option<Canonical>,
    pub operation: Option<ExampleScenarioProcessStepOperation>,
    pub alternative: Option<Vec<ExampleScenarioProcessStepAlternative>>,
    pub pause: Option<Boolean>,
}

/// Choice of types for the versionAlgorithm\[x\] field in ExampleScenario
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ExampleScenarioVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<ExampleScenarioVersionAlgorithm>,
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
    pub actor: Option<Vec<ExampleScenarioActor>>,
    pub instance: Option<Vec<ExampleScenarioInstance>>,
    pub process: Option<Vec<ExampleScenarioProcess>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExampleScenarioActor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub key: String,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub title: String,
    pub description: Option<Markdown>,
    pub definition: Option<Canonical>,
}

/// Choice of types for the timing\[x\] field in ExplanationOfBenefitSupportingInfo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "timing")]
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
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "valueIdentifier")]
    Identifier(Identifier),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "timing,value")]
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

/// Choice of types for the diagnosis\[x\] field in ExplanationOfBenefitDiagnosis
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "diagnosis")]
pub enum ExplanationOfBenefitDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "diagnosis")]
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
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitProcessNote {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<PositiveInt>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub text: Option<String>,
    pub language: Option<CodeableConcept>,
}

/// Choice of types for the serviced\[x\] field in ExplanationOfBenefitItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "serviced")]
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
#[fhir_choice_element(base_name = "location")]
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
#[fhir_resource(choice_elements = "serviced,location")]
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
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub request: Option<Vec<Reference>>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub serviced: Option<ExplanationOfBenefitItemServiced>,
    #[fhir_serde(flatten)]
    pub location: Option<ExplanationOfBenefitItemLocation>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<ExplanationOfBenefitItemBodySite>>,
    pub encounter: Option<Vec<Reference>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ExplanationOfBenefitItemReviewOutcome>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub detail: Option<Vec<ExplanationOfBenefitItemDetail>>,
}

/// Choice of types for the procedure\[x\] field in ExplanationOfBenefitProcedure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "procedure")]
pub enum ExplanationOfBenefitProcedureProcedure {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "procedureReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "procedure")]
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
pub struct ExplanationOfBenefitItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ExplanationOfBenefitItemReviewOutcome>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ExplanationOfBenefitItemReviewOutcome>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitItemDetailSubDetail>>,
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
    pub specialty: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitAddItemBodySite {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub site: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
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
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
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
    pub provider: Option<Reference>,
    pub priority: Option<CodeableConcept>,
    #[fhir_serde(rename = "fundsReserveRequested")]
    pub funds_reserve_requested: Option<CodeableConcept>,
    #[fhir_serde(rename = "fundsReserve")]
    pub funds_reserve: Option<CodeableConcept>,
    pub related: Option<Vec<ExplanationOfBenefitRelated>>,
    pub prescription: Option<Reference>,
    #[fhir_serde(rename = "originalPrescription")]
    pub original_prescription: Option<Reference>,
    pub event: Option<Vec<ExplanationOfBenefitEvent>>,
    pub payee: Option<ExplanationOfBenefitPayee>,
    pub referral: Option<Reference>,
    pub encounter: Option<Vec<Reference>>,
    pub facility: Option<Reference>,
    pub claim: Option<Reference>,
    #[fhir_serde(rename = "claimResponse")]
    pub claim_response: Option<Reference>,
    pub outcome: Code,
    pub decision: Option<CodeableConcept>,
    pub disposition: Option<String>,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[fhir_serde(rename = "preAuthRefPeriod")]
    pub pre_auth_ref_period: Option<Vec<Period>>,
    #[fhir_serde(rename = "diagnosisRelatedGroup")]
    pub diagnosis_related_group: Option<CodeableConcept>,
    #[fhir_serde(rename = "careTeam")]
    pub care_team: Option<Vec<ExplanationOfBenefitCareTeam>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<ExplanationOfBenefitSupportingInfo>>,
    pub diagnosis: Option<Vec<ExplanationOfBenefitDiagnosis>>,
    pub procedure: Option<Vec<ExplanationOfBenefitProcedure>>,
    pub precedence: Option<PositiveInt>,
    pub insurance: Option<Vec<ExplanationOfBenefitInsurance>>,
    pub accident: Option<ExplanationOfBenefitAccident>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
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
pub struct ExplanationOfBenefitTotal {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
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
    pub quantity: Option<Quantity>,
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
pub struct ExplanationOfBenefitItemBodySite {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub site: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
}

/// Choice of types for the serviced\[x\] field in ExplanationOfBenefitAddItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "serviced")]
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
#[fhir_choice_element(base_name = "location")]
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
#[fhir_resource(choice_elements = "serviced,location")]
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
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub provider: Option<Vec<Reference>>,
    pub revenue: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub request: Option<Vec<Reference>>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub serviced: Option<ExplanationOfBenefitAddItemServiced>,
    #[fhir_serde(flatten)]
    pub location: Option<ExplanationOfBenefitAddItemLocation>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<ExplanationOfBenefitAddItemBodySite>>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ExplanationOfBenefitItemReviewOutcome>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub detail: Option<Vec<ExplanationOfBenefitAddItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitAddItemDetailSubDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ExplanationOfBenefitItemReviewOutcome>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExplanationOfBenefitAddItemDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "traceNumber")]
    pub trace_number: Option<Vec<Identifier>>,
    pub revenue: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    #[fhir_serde(rename = "productOrServiceEnd")]
    pub product_or_service_end: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "patientPaid")]
    pub patient_paid: Option<Money>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub tax: Option<Money>,
    pub net: Option<Money>,
    #[fhir_serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    #[fhir_serde(rename = "reviewOutcome")]
    pub review_outcome: Option<ExplanationOfBenefitItemReviewOutcome>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[fhir_serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitAddItemDetailSubDetail>>,
}

/// Choice of types for the allowed\[x\] field in ExplanationOfBenefitBenefitBalanceFinancial
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "allowed")]
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
#[fhir_choice_element(base_name = "used")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialUsed {
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "usedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "usedMoney")]
    Money(Money),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "allowed,used")]
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

/// Choice of types for the when\[x\] field in ExplanationOfBenefitEvent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "when")]
pub enum ExplanationOfBenefitEventWhen {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "whenDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "whenPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "when")]
pub struct ExplanationOfBenefitEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub when: Option<ExplanationOfBenefitEventWhen>,
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
pub struct ExplanationOfBenefitItemReviewOutcome {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub decision: Option<CodeableConcept>,
    pub reason: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<String>,
    #[fhir_serde(rename = "preAuthPeriod")]
    pub pre_auth_period: Option<Period>,
}

/// Choice of types for the location\[x\] field in ExplanationOfBenefitAccident
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "location")]
pub enum ExplanationOfBenefitAccidentLocation {
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "location")]
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

/// Choice of types for the onset\[x\] field in FamilyMemberHistoryCondition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "onset")]
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
#[fhir_resource(choice_elements = "onset")]
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

/// Choice of types for the performed\[x\] field in FamilyMemberHistoryProcedure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "performed")]
pub enum FamilyMemberHistoryProcedurePerformed {
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "performedAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "performedRange")]
    Range(Range),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "performedPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "performedString")]
    String(String),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "performedDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "performed")]
pub struct FamilyMemberHistoryProcedure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub outcome: Option<CodeableConcept>,
    #[fhir_serde(rename = "contributedToDeath")]
    pub contributed_to_death: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub performed: Option<FamilyMemberHistoryProcedurePerformed>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct FamilyMemberHistoryParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

/// Choice of types for the born\[x\] field in FamilyMemberHistory
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "born")]
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
#[fhir_choice_element(base_name = "age")]
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
#[fhir_choice_element(base_name = "deceased")]
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
#[fhir_resource(choice_elements = "born,age,deceased")]
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
    pub status: Code,
    #[fhir_serde(rename = "dataAbsentReason")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub patient: Reference,
    pub date: Option<DateTime>,
    pub participant: Option<Vec<FamilyMemberHistoryParticipant>>,
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
    pub reason: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
    pub condition: Option<Vec<FamilyMemberHistoryCondition>>,
    pub procedure: Option<Vec<FamilyMemberHistoryProcedure>>,
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
    pub status: Option<Code>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    pub period: Option<Period>,
    pub encounter: Option<Reference>,
    pub author: Option<Reference>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct FormularyItem {
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
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GenomicStudyAnalysis {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "methodType")]
    pub method_type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "changeType")]
    pub change_type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "genomeBuild")]
    pub genome_build: Option<CodeableConcept>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Canonical>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Uri>,
    pub title: Option<String>,
    pub focus: Option<Vec<Reference>>,
    pub specimen: Option<Vec<Reference>>,
    pub date: Option<DateTime>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "protocolPerformed")]
    pub protocol_performed: Option<Reference>,
    #[fhir_serde(rename = "regionsStudied")]
    pub regions_studied: Option<Vec<Reference>>,
    #[fhir_serde(rename = "regionsCalled")]
    pub regions_called: Option<Vec<Reference>>,
    pub input: Option<Vec<GenomicStudyAnalysisInput>>,
    pub output: Option<Vec<GenomicStudyAnalysisOutput>>,
    pub performer: Option<Vec<GenomicStudyAnalysisPerformer>>,
    pub device: Option<Vec<GenomicStudyAnalysisDevice>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GenomicStudyAnalysisOutput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub file: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GenomicStudyAnalysisDevice {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub device: Option<Reference>,
    pub function: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GenomicStudy {
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
    pub r#type: Option<Vec<CodeableConcept>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "startDate")]
    pub start_date: Option<DateTime>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub referrer: Option<Reference>,
    pub interpreter: Option<Vec<Reference>>,
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Canonical>,
    #[fhir_serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Uri>,
    pub note: Option<Vec<Annotation>>,
    pub description: Option<Markdown>,
    pub analysis: Option<Vec<GenomicStudyAnalysis>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GenomicStudyAnalysisPerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub actor: Option<Reference>,
    pub role: Option<CodeableConcept>,
}

/// Choice of types for the generatedBy\[x\] field in GenomicStudyAnalysisInput
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "generatedBy")]
pub enum GenomicStudyAnalysisInputGeneratedBy {
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "generatedByIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "generatedByReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "generatedBy")]
pub struct GenomicStudyAnalysisInput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub file: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub generated_by: Option<GenomicStudyAnalysisInputGeneratedBy>,
}

/// Choice of types for the detail\[x\] field in GoalTarget
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "detail")]
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
#[fhir_choice_element(base_name = "due")]
pub enum GoalTargetDue {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "dueDate")]
    Date(Date),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "dueDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "detail,due")]
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
#[fhir_choice_element(base_name = "start")]
pub enum GoalStart {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "startDate")]
    Date(Date),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "startCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "start")]
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
    pub continuous: Option<Boolean>,
    pub priority: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub subject: Reference,
    #[fhir_serde(flatten)]
    pub start: Option<GoalStart>,
    pub acceptance: Option<Vec<GoalAcceptance>>,
    pub target: Option<Vec<GoalTarget>>,
    #[fhir_serde(rename = "statusDate")]
    pub status_date: Option<Date>,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub source: Option<Reference>,
    pub addresses: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GoalAcceptance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub participant: Reference,
    pub status: Option<Code>,
    pub priority: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GraphDefinitionNode {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "nodeId")]
    pub node_id: Id,
    pub description: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub profile: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GraphDefinitionLinkCompartment {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub rule: Code,
    pub code: Code,
    pub expression: Option<String>,
    pub description: Option<String>,
}

/// Choice of types for the versionAlgorithm\[x\] field in GraphDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum GraphDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<GraphDefinitionVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    pub start: Option<Id>,
    pub node: Option<Vec<GraphDefinitionNode>>,
    pub link: Option<Vec<GraphDefinitionLink>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GraphDefinitionLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub min: Option<Integer>,
    pub max: Option<String>,
    #[fhir_serde(rename = "sourceId")]
    pub source_id: Id,
    pub path: Option<String>,
    #[fhir_serde(rename = "sliceName")]
    pub slice_name: Option<String>,
    #[fhir_serde(rename = "targetId")]
    pub target_id: Id,
    pub params: Option<String>,
    pub compartment: Option<Vec<GraphDefinitionLinkCompartment>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in Group
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum GroupVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<GroupVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Option<Code>,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub membership: Code,
    pub code: Option<CodeableConcept>,
    pub quantity: Option<UnsignedInt>,
    #[fhir_serde(rename = "managingEntity")]
    pub managing_entity: Option<Reference>,
    #[fhir_serde(rename = "combinationMethod")]
    pub combination_method: Option<Code>,
    #[fhir_serde(rename = "combinationThreshold")]
    pub combination_threshold: Option<PositiveInt>,
    pub characteristic: Option<Vec<GroupCharacteristic>>,
    pub member: Option<Vec<GroupMember>>,
}

/// Choice of types for the value\[x\] field in GroupCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "valueExpression")]
    Expression(Expression),
}

/// Choice of types for the determinedBy\[x\] field in GroupCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "determinedBy")]
pub enum GroupCharacteristicDeterminedBy {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "determinedByReference")]
    Reference(Reference),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "determinedByExpression")]
    Expression(Expression),
}

/// Choice of types for the instances\[x\] field in GroupCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "instances")]
pub enum GroupCharacteristicInstances {
    /// Variant accepting the UnsignedInt type.
    #[fhir_serde(rename = "instancesUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "instancesRange")]
    Range(Range),
}

/// Choice of types for the duration\[x\] field in GroupCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "duration")]
pub enum GroupCharacteristicDuration {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "durationDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "durationRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value,determinedBy,instances,duration")]
pub struct GroupCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<GroupCharacteristicValue>,
    pub exclude: Boolean,
    pub description: Option<Markdown>,
    pub method: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub determined_by: Option<GroupCharacteristicDeterminedBy>,
    pub offset: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub instances: Option<GroupCharacteristicInstances>,
    #[fhir_serde(flatten)]
    pub duration: Option<GroupCharacteristicDuration>,
    pub period: Option<Period>,
    pub timing: Option<Vec<RelativeTime>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct GroupMember {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub entity: Reference,
    pub involvement: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub inactive: Option<Boolean>,
}

/// Choice of types for the module\[x\] field in GuidanceResponse
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "module")]
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
#[fhir_resource(choice_elements = "module")]
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
    pub reason: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "evaluationMessage")]
    pub evaluation_message: Option<Reference>,
    #[fhir_serde(rename = "outputParameters")]
    pub output_parameters: Option<Reference>,
    pub result: Option<Vec<Reference>>,
    #[fhir_serde(rename = "dataRequirement")]
    pub data_requirement: Option<Vec<DataRequirement>>,
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
    #[fhir_serde(rename = "offeredIn")]
    pub offered_in: Option<Vec<Reference>>,
    pub category: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    pub name: Option<String>,
    pub comment: Option<Markdown>,
    #[fhir_serde(rename = "extraDetails")]
    pub extra_details: Option<Markdown>,
    pub photo: Option<Attachment>,
    pub contact: Option<Vec<ExtendedContactDetail>>,
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
    #[fhir_serde(rename = "referralRequired")]
    pub referral_required: Option<Boolean>,
    #[fhir_serde(rename = "appointmentRequired")]
    pub appointment_required: Option<Boolean>,
    pub availability: Option<Availability>,
    pub endpoint: Option<Vec<Reference>>,
}

/// Choice of types for the value\[x\] field in HealthcareServiceEligibility
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum HealthcareServiceEligibilityValue {
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
#[fhir_resource(choice_elements = "value")]
pub struct HealthcareServiceEligibility {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<HealthcareServiceEligibilityValue>,
    pub comment: Option<Markdown>,
    pub period: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingSelectionImageRegion3D {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "regionType")]
    pub region_type: Code,
    pub coordinate: Option<Vec<Decimal>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingSelectionPerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingSelectionInstanceImageRegion2D {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "regionType")]
    pub region_type: Code,
    pub coordinate: Option<Vec<Decimal>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingSelection {
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
    pub modality: Option<CodeableConcept>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub issued: Option<Instant>,
    pub performer: Option<Vec<ImagingSelectionPerformer>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Reference>,
    #[fhir_serde(rename = "studyUid")]
    pub study_uid: Option<Id>,
    #[fhir_serde(rename = "seriesUid")]
    pub series_uid: Option<Id>,
    #[fhir_serde(rename = "seriesNumber")]
    pub series_number: Option<UnsignedInt>,
    #[fhir_serde(rename = "frameOfReferenceUid")]
    pub frame_of_reference_uid: Option<Id>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableReference>>,
    pub focus: Option<Vec<Reference>>,
    pub endpoint: Option<Vec<Reference>>,
    pub instance: Option<Vec<ImagingSelectionInstance>>,
    #[fhir_serde(rename = "imageRegion3D")]
    pub image_region3_d: Option<Vec<ImagingSelectionImageRegion3D>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingSelectionInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    pub number: Option<UnsignedInt>,
    #[fhir_serde(rename = "sopClass")]
    pub sop_class: Option<Oid>,
    pub subset: Option<Vec<String>>,
    #[fhir_serde(rename = "imageRegion2D")]
    pub image_region2_d: Option<Vec<ImagingSelectionInstanceImageRegion2D>>,
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
    pub modality: Option<Vec<CodeableConcept>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub started: Option<DateTime>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub referrer: Option<Reference>,
    pub endpoint: Option<Vec<Reference>>,
    pub location: Option<Reference>,
    pub reason: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
    pub description: Option<String>,
    #[fhir_serde(rename = "numberOfSeries")]
    pub number_of_series: Option<UnsignedInt>,
    #[fhir_serde(rename = "numberOfInstances")]
    pub number_of_instances: Option<UnsignedInt>,
    pub series: Option<Vec<ImagingStudySeries>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImagingStudySeries {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    pub number: Option<UnsignedInt>,
    pub modality: CodeableConcept,
    pub description: Option<String>,
    #[fhir_serde(rename = "numberOfInstances")]
    pub number_of_instances: Option<UnsignedInt>,
    pub endpoint: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableReference>,
    pub laterality: Option<CodeableConcept>,
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
    pub sop_class: Oid,
    pub number: Option<UnsignedInt>,
    pub title: Option<String>,
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
pub struct ImmunizationProgramEligibility {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub program: CodeableConcept,
    #[fhir_serde(rename = "programStatus")]
    pub program_status: CodeableConcept,
}

/// Choice of types for the occurrence\[x\] field in Immunization
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
pub enum ImmunizationOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "occurrenceString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "occurrence")]
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
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    #[fhir_serde(rename = "vaccineCode")]
    pub vaccine_code: CodeableConcept,
    #[fhir_serde(rename = "administeredProduct")]
    pub administered_product: Option<CodeableReference>,
    pub manufacturer: Option<CodeableReference>,
    #[fhir_serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    #[fhir_serde(rename = "expirationDate")]
    pub expiration_date: Option<Date>,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<ImmunizationOccurrence>,
    #[fhir_serde(rename = "primarySource")]
    pub primary_source: Option<Boolean>,
    #[fhir_serde(rename = "informationSource")]
    pub information_source: Option<CodeableReference>,
    pub location: Option<Reference>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    #[fhir_serde(rename = "doseQuantity")]
    pub dose_quantity: Option<Quantity>,
    pub performer: Option<Vec<ImmunizationPerformer>>,
    pub note: Option<Vec<Annotation>>,
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "isSubpotent")]
    pub is_subpotent: Option<Boolean>,
    #[fhir_serde(rename = "subpotentReason")]
    pub subpotent_reason: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "programEligibility")]
    pub program_eligibility: Option<Vec<ImmunizationProgramEligibility>>,
    #[fhir_serde(rename = "fundingSource")]
    pub funding_source: Option<CodeableConcept>,
    pub reaction: Option<Vec<ImmunizationReaction>>,
    #[fhir_serde(rename = "protocolApplied")]
    pub protocol_applied: Option<Vec<ImmunizationProtocolApplied>>,
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
    #[fhir_serde(rename = "doseNumber")]
    pub dose_number: Option<CodeableConcept>,
    #[fhir_serde(rename = "seriesDoses")]
    pub series_doses: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationReaction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<DateTime>,
    pub manifestation: Option<CodeableReference>,
    pub reported: Option<Boolean>,
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
    pub description: Option<Markdown>,
    pub series: Option<String>,
    #[fhir_serde(rename = "doseNumber")]
    pub dose_number: Option<CodeableConcept>,
    #[fhir_serde(rename = "seriesDoses")]
    pub series_doses: Option<CodeableConcept>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImmunizationRecommendationRecommendation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "vaccineCode")]
    pub vaccine_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "targetDisease")]
    pub target_disease: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "contraindicatedVaccineCode")]
    pub contraindicated_vaccine_code: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "forecastStatus")]
    pub forecast_status: CodeableConcept,
    #[fhir_serde(rename = "forecastReason")]
    pub forecast_reason: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "dateCriterion")]
    pub date_criterion: Option<Vec<ImmunizationRecommendationRecommendationDateCriterion>>,
    pub description: Option<Markdown>,
    pub series: Option<String>,
    #[fhir_serde(rename = "doseNumber")]
    pub dose_number: Option<CodeableConcept>,
    #[fhir_serde(rename = "seriesDoses")]
    pub series_doses: Option<CodeableConcept>,
    #[fhir_serde(rename = "supportingImmunization")]
    pub supporting_immunization: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingPatientInformation")]
    pub supporting_patient_information: Option<Vec<Reference>>,
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
pub struct ImplementationGuideDependsOn {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Canonical,
    #[fhir_serde(rename = "packageId")]
    pub package_id: Option<Id>,
    pub version: Option<String>,
    pub reason: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideManifestResource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[fhir_serde(rename = "isExample")]
    pub is_example: Option<Boolean>,
    pub profile: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "relativePath")]
    pub relative_path: Option<Url>,
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
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "isExample")]
    pub is_example: Option<Boolean>,
    pub profile: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "groupingId")]
    pub grouping_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDefinitionGrouping {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub description: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ImplementationGuideDefinitionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Coding,
    pub value: String,
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
pub struct ImplementationGuideManifestPage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub title: Option<String>,
    pub anchor: Option<Vec<String>>,
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

/// Choice of types for the versionAlgorithm\[x\] field in ImplementationGuide
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ImplementationGuideVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<ImplementationGuideVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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

/// Choice of types for the source\[x\] field in ImplementationGuideDefinitionPage
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "source")]
pub enum ImplementationGuideDefinitionPageSource {
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "sourceUrl")]
    Url(Url),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "sourceString")]
    String(String),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "sourceMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "source")]
pub struct ImplementationGuideDefinitionPage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub source: Option<ImplementationGuideDefinitionPageSource>,
    pub name: Url,
    pub title: String,
    pub generation: Code,
    pub page: Option<Vec<ImplementationGuideDefinitionPage>>,
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

/// Choice of types for the presentation\[x\] field in IngredientSubstanceStrength
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "presentation")]
pub enum IngredientSubstanceStrengthPresentation {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "presentationRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "presentationRatioRange")]
    RatioRange(RatioRange),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "presentationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "presentationQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the concentration\[x\] field in IngredientSubstanceStrength
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "concentration")]
pub enum IngredientSubstanceStrengthConcentration {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "concentrationRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "concentrationRatioRange")]
    RatioRange(RatioRange),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "concentrationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "concentrationQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "presentation,concentration")]
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
    pub basis: Option<CodeableConcept>,
    #[fhir_serde(rename = "measurementPoint")]
    pub measurement_point: Option<String>,
    pub country: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "referenceStrength")]
    pub reference_strength: Option<Vec<IngredientSubstanceStrengthReferenceStrength>>,
}

/// Choice of types for the strength\[x\] field in IngredientSubstanceStrengthReferenceStrength
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "strength")]
pub enum IngredientSubstanceStrengthReferenceStrengthStrength {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "strengthRatio")]
    Ratio(Ratio),
    /// Variant accepting the RatioRange type.
    #[fhir_serde(rename = "strengthRatioRange")]
    RatioRange(RatioRange),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "strengthQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "strength")]
pub struct IngredientSubstanceStrengthReferenceStrength {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: CodeableReference,
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
    pub group: Option<CodeableConcept>,
    #[fhir_serde(rename = "allergenicIndicator")]
    pub allergenic_indicator: Option<Boolean>,
    pub comment: Option<Markdown>,
    pub manufacturer: Option<Vec<IngredientManufacturer>>,
    pub substance: IngredientSubstance,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanSpecificCostBenefitCost {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub applicability: Option<CodeableConcept>,
    pub qualifier: Option<Vec<CodeableConcept>>,
    pub value: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanGeneralCost {
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
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub product: Option<Reference>,
    #[fhir_serde(rename = "coverageArea")]
    pub coverage_area: Option<Vec<Reference>>,
    pub network: Option<Vec<Reference>>,
    #[fhir_serde(rename = "generalCost")]
    pub general_cost: Option<Vec<InsurancePlanGeneralCost>>,
    #[fhir_serde(rename = "specificCost")]
    pub specific_cost: Option<Vec<InsurancePlanSpecificCost>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanSpecificCost {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub benefit: Option<Vec<InsurancePlanSpecificCostBenefit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsurancePlanSpecificCostBenefit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub cost: Option<Vec<InsurancePlanSpecificCostBenefitCost>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsuranceProduct {
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
    pub contact: Option<Vec<ExtendedContactDetail>>,
    pub endpoint: Option<Vec<Reference>>,
    pub network: Option<Vec<Reference>>,
    pub coverage: Option<Vec<InsuranceProductCoverage>>,
    pub related: Option<Vec<InsuranceProductRelated>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsuranceProductCoverage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub network: Option<Vec<Reference>>,
    pub benefit: Option<Vec<InsuranceProductCoverageBenefit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsuranceProductRelated {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub product: Option<Reference>,
    pub relationship: Option<CodeableConcept>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsuranceProductCoverageBenefit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub requirement: Option<String>,
    pub limit: Option<Vec<InsuranceProductCoverageBenefitLimit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InsuranceProductCoverageBenefitLimit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<Quantity>,
    pub code: Option<CodeableConcept>,
}

/// Choice of types for the value\[x\] field in InventoryItemCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum InventoryItemCharacteristicValue {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Decimal type.
    #[fhir_serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Annotation type.
    #[fhir_serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Address type.
    #[fhir_serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct InventoryItemCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "characteristicType")]
    pub characteristic_type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<InventoryItemCharacteristicValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryItemInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    pub expiry: Option<DateTime>,
    pub subject: Option<Reference>,
    pub location: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryItemDescription {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<Code>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryItemAssociation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "associationType")]
    pub association_type: CodeableConcept,
    #[fhir_serde(rename = "relatedItem")]
    pub related_item: Reference,
    pub quantity: Ratio,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryItemResponsibleOrganization {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: CodeableConcept,
    pub organization: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryItem {
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
    pub code: Option<Vec<CodeableConcept>>,
    pub name: Option<Vec<InventoryItemName>>,
    #[fhir_serde(rename = "responsibleOrganization")]
    pub responsible_organization: Option<Vec<InventoryItemResponsibleOrganization>>,
    pub description: Option<InventoryItemDescription>,
    #[fhir_serde(rename = "inventoryStatus")]
    pub inventory_status: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "baseUnit")]
    pub base_unit: Option<CodeableConcept>,
    #[fhir_serde(rename = "netContent")]
    pub net_content: Option<Quantity>,
    pub association: Option<Vec<InventoryItemAssociation>>,
    pub characteristic: Option<Vec<InventoryItemCharacteristic>>,
    pub instance: Option<InventoryItemInstance>,
    #[fhir_serde(rename = "productReference")]
    pub product_reference: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryItemName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "nameType")]
    pub name_type: Coding,
    pub language: Code,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryReport {
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
    #[fhir_serde(rename = "countType")]
    pub count_type: Code,
    #[fhir_serde(rename = "operationType")]
    pub operation_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "operationTypeReason")]
    pub operation_type_reason: Option<CodeableConcept>,
    #[fhir_serde(rename = "reportedDateTime")]
    pub reported_date_time: DateTime,
    pub reporter: Option<Reference>,
    #[fhir_serde(rename = "reportingPeriod")]
    pub reporting_period: Option<Period>,
    #[fhir_serde(rename = "inventoryListing")]
    pub inventory_listing: Option<Vec<InventoryReportInventoryListing>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryReportInventoryListing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub location: Option<Reference>,
    #[fhir_serde(rename = "itemStatus")]
    pub item_status: Option<CodeableConcept>,
    #[fhir_serde(rename = "countingDateTime")]
    pub counting_date_time: Option<DateTime>,
    pub item: Option<Vec<InventoryReportInventoryListingItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct InventoryReportInventoryListingItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    pub quantity: Quantity,
    pub item: CodeableReference,
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

/// Choice of types for the period\[x\] field in Invoice
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "period")]
pub enum InvoicePeriod {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "periodDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "periodPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "period")]
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
    pub creation: Option<DateTime>,
    #[fhir_serde(flatten)]
    pub period: Option<InvoicePeriod>,
    pub participant: Option<Vec<InvoiceParticipant>>,
    pub issuer: Option<Reference>,
    pub account: Option<Reference>,
    #[fhir_serde(rename = "lineItem")]
    pub line_item: Option<Vec<InvoiceLineItem>>,
    #[fhir_serde(rename = "totalPriceComponent")]
    pub total_price_component: Option<Vec<MonetaryComponent>>,
    #[fhir_serde(rename = "totalNet")]
    pub total_net: Option<Money>,
    #[fhir_serde(rename = "totalGross")]
    pub total_gross: Option<Money>,
    #[fhir_serde(rename = "paymentTerms")]
    pub payment_terms: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the serviced\[x\] field in InvoiceLineItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "serviced")]
pub enum InvoiceLineItemServiced {
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the chargeItem\[x\] field in InvoiceLineItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "chargeItem")]
pub enum InvoiceLineItemChargeItem {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "chargeItemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "chargeItemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "serviced,chargeItem")]
pub struct InvoiceLineItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<PositiveInt>,
    #[fhir_serde(flatten)]
    pub serviced: Option<InvoiceLineItemServiced>,
    #[fhir_serde(flatten)]
    pub charge_item: Option<InvoiceLineItemChargeItem>,
    #[fhir_serde(rename = "priceComponent")]
    pub price_component: Option<Vec<MonetaryComponent>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in Library
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum LibraryVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

/// Choice of types for the subject\[x\] field in Library
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
pub enum LibrarySubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm,subject")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<LibraryVersionAlgorithm>,
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
    pub usage: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
    pub subject: Option<Vec<Reference>>,
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
    pub description: Option<Markdown>,
    pub mode: Option<Code>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub contact: Option<Vec<ExtendedContactDetail>>,
    pub address: Option<Address>,
    pub form: Option<CodeableConcept>,
    pub position: Option<LocationPosition>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Reference>,
    pub characteristic: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "hoursOfOperation")]
    pub hours_of_operation: Option<Availability>,
    #[fhir_serde(rename = "virtualService")]
    pub virtual_service: Option<Vec<VirtualServiceDetail>>,
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

/// Choice of types for the value\[x\] field in ManufacturedItemDefinitionProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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
    pub name: Option<String>,
    #[fhir_serde(rename = "manufacturedDoseForm")]
    pub manufactured_dose_form: CodeableConcept,
    #[fhir_serde(rename = "unitOfPresentation")]
    pub unit_of_presentation: Option<CodeableConcept>,
    pub manufacturer: Option<Vec<Reference>>,
    #[fhir_serde(rename = "marketingStatus")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    pub ingredient: Option<Vec<CodeableConcept>>,
    pub property: Option<Vec<ManufacturedItemDefinitionProperty>>,
    pub component: Option<Vec<ManufacturedItemDefinitionComponent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ManufacturedItemDefinitionComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub function: Option<Vec<CodeableConcept>>,
    pub amount: Option<Vec<Quantity>>,
    pub constituent: Option<Vec<ManufacturedItemDefinitionComponentConstituent>>,
    pub property: Option<Vec<ManufacturedItemDefinitionProperty>>,
    pub component: Option<Vec<ManufacturedItemDefinitionComponent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ManufacturedItemDefinitionComponentConstituent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub amount: Option<Vec<Quantity>>,
    pub location: Option<Vec<CodeableConcept>>,
    pub function: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "hasIngredient")]
    pub has_ingredient: Option<Vec<CodeableReference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureGroupPopulation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub criteria: Option<Expression>,
    #[fhir_serde(rename = "groupDefinition")]
    pub group_definition: Option<Reference>,
    #[fhir_serde(rename = "inputPopulationId")]
    pub input_population_id: Option<String>,
    #[fhir_serde(rename = "aggregateMethod")]
    pub aggregate_method: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureTerm {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub definition: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureGroupStratifierComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub criteria: Option<Expression>,
    #[fhir_serde(rename = "groupDefinition")]
    pub group_definition: Option<Reference>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub unit: Option<String>,
}

/// Choice of types for the versionAlgorithm\[x\] field in Measure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum MeasureVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

/// Choice of types for the subject\[x\] field in Measure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
pub enum MeasureSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm,subject")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<MeasureVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub subject: Option<MeasureSubject>,
    pub basis: Option<Code>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[fhir_serde(rename = "reportingFrequency")]
    pub reporting_frequency: Option<Quantity>,
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
    #[fhir_serde(rename = "scoringUnit")]
    pub scoring_unit: Option<CodeableConcept>,
    #[fhir_serde(rename = "scoringPrecision")]
    pub scoring_precision: Option<PositiveInt>,
    #[fhir_serde(rename = "compositeScoring")]
    pub composite_scoring: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "riskAdjustment")]
    pub risk_adjustment: Option<Markdown>,
    #[fhir_serde(rename = "rateAggregation")]
    pub rate_aggregation: Option<Markdown>,
    pub rationale: Option<Markdown>,
    #[fhir_serde(rename = "clinicalRecommendationStatement")]
    pub clinical_recommendation_statement: Option<Markdown>,
    #[fhir_serde(rename = "improvementNotation")]
    pub improvement_notation: Option<CodeableConcept>,
    #[fhir_serde(rename = "improvementNotationGuidance")]
    pub improvement_notation_guidance: Option<Markdown>,
    pub term: Option<Vec<MeasureTerm>>,
    pub guidance: Option<Markdown>,
    pub group: Option<Vec<MeasureGroup>>,
    #[fhir_serde(rename = "supplementalData")]
    pub supplemental_data: Option<Vec<MeasureSupplementalData>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureGroupStratifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub criteria: Option<Expression>,
    #[fhir_serde(rename = "groupDefinition")]
    pub group_definition: Option<Reference>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub unit: Option<String>,
    pub component: Option<Vec<MeasureGroupStratifierComponent>>,
}

/// Choice of types for the subject\[x\] field in MeasureGroup
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
pub enum MeasureGroupSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "subject")]
pub struct MeasureGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[fhir_serde(flatten)]
    pub subject: Option<MeasureGroupSubject>,
    pub basis: Option<Code>,
    pub scoring: Option<CodeableConcept>,
    #[fhir_serde(rename = "scoringUnit")]
    pub scoring_unit: Option<CodeableConcept>,
    #[fhir_serde(rename = "scoringPrecision")]
    pub scoring_precision: Option<PositiveInt>,
    #[fhir_serde(rename = "compositeScoring")]
    pub composite_scoring: Option<CodeableConcept>,
    pub component: Option<Vec<MeasureGroupComponent>>,
    #[fhir_serde(rename = "rateAggregation")]
    pub rate_aggregation: Option<Markdown>,
    #[fhir_serde(rename = "improvementNotation")]
    pub improvement_notation: Option<CodeableConcept>,
    #[fhir_serde(rename = "improvementNotationGuidance")]
    pub improvement_notation_guidance: Option<Markdown>,
    pub library: Option<Vec<Canonical>>,
    pub population: Option<Vec<MeasureGroupPopulation>>,
    pub stratifier: Option<Vec<MeasureGroupStratifier>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureGroupComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub measure: Option<Canonical>,
    #[fhir_serde(rename = "groupId")]
    pub group_id: Option<String>,
    pub weight: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureSupplementalData {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: Option<CodeableConcept>,
    pub usage: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
    pub criteria: Expression,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub unit: Option<String>,
}

/// Choice of types for the measureScore\[x\] field in MeasureReportGroup
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "measureScore")]
pub enum MeasureReportGroupMeasureScore {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "measureScoreQuantity")]
    Quantity(Quantity),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "measureScoreDateTime")]
    DateTime(DateTime),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "measureScoreCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "measureScorePeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "measureScoreRange")]
    Range(Range),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "measureScoreDuration")]
    Duration(Duration),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "measureScoreBoolean")]
    Boolean(Boolean),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "measureScore")]
pub struct MeasureReportGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    #[fhir_serde(rename = "calculatedDate")]
    pub calculated_date: Option<DateTime>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub subject: Option<Reference>,
    pub scoring: Option<CodeableConcept>,
    #[fhir_serde(rename = "improvementNotation")]
    pub improvement_notation: Option<CodeableConcept>,
    #[fhir_serde(rename = "improvementNotationGuidance")]
    pub improvement_notation_guidance: Option<Markdown>,
    pub population: Option<Vec<MeasureReportGroupPopulation>>,
    #[fhir_serde(flatten)]
    pub measure_score: Option<MeasureReportGroupMeasureScore>,
    pub stratifier: Option<Vec<MeasureReportGroupStratifier>>,
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
    pub category: Option<CodeableConcept>,
    pub messages: Option<Reference>,
    pub status: Code,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(rename = "dataUpdateType")]
    pub data_update_type: Option<Code>,
    pub measure: Option<Canonical>,
    pub subject: Option<Reference>,
    pub date: Option<DateTime>,
    pub reporter: Option<Reference>,
    #[fhir_serde(rename = "reportingVendor")]
    pub reporting_vendor: Option<Reference>,
    pub location: Option<Vec<Reference>>,
    pub period: Period,
    #[fhir_serde(rename = "inputParameters")]
    pub input_parameters: Option<Reference>,
    pub scoring: Option<CodeableConcept>,
    #[fhir_serde(rename = "improvementNotation")]
    pub improvement_notation: Option<CodeableConcept>,
    pub group: Option<Vec<MeasureReportGroup>>,
    #[fhir_serde(rename = "supplementalData")]
    pub supplemental_data: Option<Vec<Reference>>,
    #[fhir_serde(rename = "evaluatedResource")]
    pub evaluated_resource: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroupStratifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub stratum: Option<Vec<MeasureReportGroupStratifierStratum>>,
}

/// Choice of types for the value\[x\] field in MeasureReportGroupStratifierStratum
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum MeasureReportGroupStratifierStratumValue {
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

/// Choice of types for the measureScore\[x\] field in MeasureReportGroupStratifierStratum
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "measureScore")]
pub enum MeasureReportGroupStratifierStratumMeasureScore {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "measureScoreQuantity")]
    Quantity(Quantity),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "measureScoreDateTime")]
    DateTime(DateTime),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "measureScoreCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "measureScorePeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "measureScoreRange")]
    Range(Range),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "measureScoreDuration")]
    Duration(Duration),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "measureScoreBoolean")]
    Boolean(Boolean),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value,measureScore")]
pub struct MeasureReportGroupStratifierStratum {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<MeasureReportGroupStratifierStratumValue>,
    pub component: Option<Vec<MeasureReportGroupStratifierStratumComponent>>,
    pub population: Option<Vec<MeasureReportGroupStratifierStratumPopulation>>,
    #[fhir_serde(flatten)]
    pub measure_score: Option<MeasureReportGroupStratifierStratumMeasureScore>,
}

/// Choice of types for the value\[x\] field in MeasureReportGroupStratifierStratumComponent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum MeasureReportGroupStratifierStratumComponentValue {
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
#[fhir_resource(choice_elements = "value")]
pub struct MeasureReportGroupStratifierStratumComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: CodeableConcept,
    pub description: Option<Markdown>,
    #[fhir_serde(flatten)]
    pub value: Option<MeasureReportGroupStratifierStratumComponentValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroupStratifierStratumPopulation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: Option<CodeableConcept>,
    pub count: Option<Integer>,
    #[fhir_serde(rename = "countQuantity")]
    pub count_quantity: Option<Quantity>,
    #[fhir_serde(rename = "subjectResults")]
    pub subject_results: Option<Reference>,
    #[fhir_serde(rename = "subjectReport")]
    pub subject_report: Option<Vec<Reference>>,
    pub subjects: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MeasureReportGroupPopulation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub count: Option<Integer>,
    #[fhir_serde(rename = "countQuantity")]
    pub count_quantity: Option<Quantity>,
    #[fhir_serde(rename = "subjectResults")]
    pub subject_results: Option<Reference>,
    #[fhir_serde(rename = "subjectReport")]
    pub subject_report: Option<Vec<Reference>>,
    pub subjects: Option<Reference>,
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
    #[fhir_serde(rename = "marketingAuthorizationHolder")]
    pub marketing_authorization_holder: Option<Reference>,
    #[fhir_serde(rename = "doseForm")]
    pub dose_form: Option<CodeableConcept>,
    #[fhir_serde(rename = "totalVolume")]
    pub total_volume: Option<Quantity>,
    pub ingredient: Option<Vec<MedicationIngredient>>,
    pub batch: Option<MedicationBatch>,
    pub definition: Option<Reference>,
}

/// Choice of types for the strength\[x\] field in MedicationIngredient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "strength")]
pub enum MedicationIngredientStrength {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "strengthRatio")]
    Ratio(Ratio),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "strengthCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "strengthQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "strength")]
pub struct MedicationIngredient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    #[fhir_serde(rename = "isActive")]
    pub is_active: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub strength: Option<MedicationIngredientStrength>,
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

/// Choice of types for the occurrence\[x\] field in MedicationAdministration
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
pub enum MedicationAdministrationOccurrence {
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
#[fhir_resource(choice_elements = "occurrence")]
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
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub category: Option<Vec<CodeableConcept>>,
    pub medication: CodeableReference,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<MedicationAdministrationOccurrence>,
    pub recorded: Option<DateTime>,
    #[fhir_serde(rename = "isSubPotent")]
    pub is_sub_potent: Option<Boolean>,
    #[fhir_serde(rename = "subPotentReason")]
    pub sub_potent_reason: Option<Vec<CodeableConcept>>,
    pub performer: Option<Vec<MedicationAdministrationPerformer>>,
    pub reason: Option<Vec<CodeableReference>>,
    pub request: Option<Reference>,
    pub device: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage: Option<MedicationAdministrationDosage>,
    #[fhir_serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationAdministrationPerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: CodeableReference,
}

/// Choice of types for the rate\[x\] field in MedicationAdministrationDosage
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "rate")]
pub enum MedicationAdministrationDosageRate {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "rateRatio")]
    Ratio(Ratio),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "rateQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "rate")]
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
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "notPerformedReason")]
    pub not_performed_reason: Option<CodeableReference>,
    #[fhir_serde(rename = "statusChanged")]
    pub status_changed: Option<DateTime>,
    pub category: Option<Vec<CodeableConcept>>,
    pub medication: CodeableReference,
    pub subject: Reference,
    pub encounter: Option<Reference>,
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
    #[fhir_serde(rename = "fillNumber")]
    pub fill_number: Option<PositiveInt>,
    pub recorded: Option<DateTime>,
    #[fhir_serde(rename = "whenPrepared")]
    pub when_prepared: Option<DateTime>,
    #[fhir_serde(rename = "whenHandedOver")]
    pub when_handed_over: Option<DateTime>,
    pub destination: Option<Reference>,
    pub receiver: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "renderedDosageInstruction")]
    pub rendered_dosage_instruction: Option<Markdown>,
    #[fhir_serde(rename = "dosageInstruction")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    pub substitution: Option<MedicationDispenseSubstitution>,
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
    pub responsible_party: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeIndicationGuideline {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub indication: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "dosingGuideline")]
    pub dosing_guideline: Option<Vec<MedicationKnowledgeIndicationGuidelineDosingGuideline>>,
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

/// Choice of types for the cost\[x\] field in MedicationKnowledgeCost
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "cost")]
pub enum MedicationKnowledgeCostCost {
    /// Variant accepting the Money type.
    #[fhir_serde(rename = "costMoney")]
    Money(Money),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "costCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "cost")]
pub struct MedicationKnowledgeCost {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "effectiveDate")]
    pub effective_date: Option<Vec<Period>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub source: Option<String>,
    #[fhir_serde(flatten)]
    pub cost: Option<MedicationKnowledgeCostCost>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgePackaging {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub cost: Option<Vec<MedicationKnowledgeCost>>,
    #[fhir_serde(rename = "packagedProduct")]
    pub packaged_product: Option<Reference>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeDefinitional {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub definition: Option<Vec<Reference>>,
    #[fhir_serde(rename = "doseForm")]
    pub dose_form: Option<CodeableConcept>,
    #[fhir_serde(rename = "intendedRoute")]
    pub intended_route: Option<Vec<CodeableConcept>>,
    pub ingredient: Option<Vec<MedicationKnowledgeDefinitionalIngredient>>,
    #[fhir_serde(rename = "drugCharacteristic")]
    pub drug_characteristic: Option<Vec<MedicationKnowledgeDefinitionalDrugCharacteristic>>,
}

/// Choice of types for the strength\[x\] field in MedicationKnowledgeDefinitionalIngredient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "strength")]
pub enum MedicationKnowledgeDefinitionalIngredientStrength {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "strengthRatio")]
    Ratio(Ratio),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "strengthCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "strengthQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "strength")]
pub struct MedicationKnowledgeDefinitionalIngredient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub strength: Option<MedicationKnowledgeDefinitionalIngredientStrength>,
}

/// Choice of types for the value\[x\] field in MedicationKnowledgeDefinitionalDrugCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum MedicationKnowledgeDefinitionalDrugCharacteristicValue {
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
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct MedicationKnowledgeDefinitionalDrugCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<MedicationKnowledgeDefinitionalDrugCharacteristicValue>,
}

/// Choice of types for the value\[x\] field in MedicationKnowledgeStorageGuidelineEnvironmentalSetting
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum MedicationKnowledgeStorageGuidelineEnvironmentalSettingValue {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct MedicationKnowledgeStorageGuidelineEnvironmentalSetting {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<MedicationKnowledgeStorageGuidelineEnvironmentalSettingValue>,
}

/// Choice of types for the source\[x\] field in MedicationKnowledgeMedicineClassification
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "source")]
pub enum MedicationKnowledgeMedicineClassificationSource {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "sourceString")]
    String(String),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "sourceUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "source")]
pub struct MedicationKnowledgeMedicineClassification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub source: Option<MedicationKnowledgeMedicineClassificationSource>,
    pub classification: Option<Vec<CodeableConcept>>,
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
    pub schedule: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "maxDispense")]
    pub max_dispense: Option<MedicationKnowledgeRegulatoryMaxDispense>,
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
    pub identifier: Option<Vec<Identifier>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<Code>,
    pub author: Option<ContactDetail>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    #[fhir_serde(rename = "relatedMedicationKnowledge")]
    pub related_medication_knowledge: Option<Vec<MedicationKnowledgeRelatedMedicationKnowledge>>,
    #[fhir_serde(rename = "associatedMedication")]
    pub associated_medication: Option<Vec<Reference>>,
    #[fhir_serde(rename = "productType")]
    pub product_type: Option<Vec<CodeableConcept>>,
    pub monograph: Option<Vec<MedicationKnowledgeMonograph>>,
    #[fhir_serde(rename = "preparationInstruction")]
    pub preparation_instruction: Option<Markdown>,
    pub cost: Option<Vec<MedicationKnowledgeCost>>,
    #[fhir_serde(rename = "monitoringProgram")]
    pub monitoring_program: Option<Vec<MedicationKnowledgeMonitoringProgram>>,
    #[fhir_serde(rename = "indicationGuideline")]
    pub indication_guideline: Option<Vec<MedicationKnowledgeIndicationGuideline>>,
    #[fhir_serde(rename = "medicineClassification")]
    pub medicine_classification: Option<Vec<MedicationKnowledgeMedicineClassification>>,
    pub packaging: Option<Vec<MedicationKnowledgePackaging>>,
    #[fhir_serde(rename = "clinicalUseIssue")]
    pub clinical_use_issue: Option<Vec<Reference>>,
    #[fhir_serde(rename = "storageGuideline")]
    pub storage_guideline: Option<Vec<MedicationKnowledgeStorageGuideline>>,
    pub regulatory: Option<Vec<MedicationKnowledgeRegulatory>>,
    pub definitional: Option<MedicationKnowledgeDefinitional>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeStorageGuideline {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Uri>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "stabilityDuration")]
    pub stability_duration: Option<Duration>,
    #[fhir_serde(rename = "environmentalSetting")]
    pub environmental_setting: Option<Vec<MedicationKnowledgeStorageGuidelineEnvironmentalSetting>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationKnowledgeIndicationGuidelineDosingGuideline {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "treatmentIntent")]
    pub treatment_intent: Option<CodeableConcept>,
    pub dosage: Option<Vec<MedicationKnowledgeIndicationGuidelineDosingGuidelineDosage>>,
    #[fhir_serde(rename = "administrationTreatment")]
    pub administration_treatment: Option<CodeableConcept>,
    #[fhir_serde(rename = "patientCharacteristic")]
    pub patient_characteristic:
        Option<Vec<MedicationKnowledgeIndicationGuidelineDosingGuidelinePatientCharacteristic>>,
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
pub struct MedicationKnowledgeIndicationGuidelineDosingGuidelineDosage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub dosage: Option<Vec<Dosage>>,
}

/// Choice of types for the value\[x\] field in MedicationKnowledgeIndicationGuidelineDosingGuidelinePatientCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum MedicationKnowledgeIndicationGuidelineDosingGuidelinePatientCharacteristicValue {
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
#[fhir_resource(choice_elements = "value")]
pub struct MedicationKnowledgeIndicationGuidelineDosingGuidelinePatientCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value:
        Option<MedicationKnowledgeIndicationGuidelineDosingGuidelinePatientCharacteristicValue>,
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
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "priorPrescription")]
    pub prior_prescription: Option<Reference>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    #[fhir_serde(rename = "statusChanged")]
    pub status_changed: Option<DateTime>,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    pub medication: CodeableReference,
    pub subject: Reference,
    #[fhir_serde(rename = "informationSource")]
    pub information_source: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub reported: Option<Boolean>,
    #[fhir_serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub performer: Option<Vec<Reference>>,
    pub device: Option<Vec<CodeableReference>>,
    pub recorder: Option<Reference>,
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "courseOfTherapyType")]
    pub course_of_therapy_type: Option<CodeableConcept>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "renderedDosageInstruction")]
    pub rendered_dosage_instruction: Option<Markdown>,
    #[fhir_serde(rename = "effectiveDosePeriod")]
    pub effective_dose_period: Option<Period>,
    #[fhir_serde(rename = "dosageInstruction")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    #[fhir_serde(rename = "dispenseRequest")]
    pub dispense_request: Option<MedicationRequestDispenseRequest>,
    pub substitution: Option<MedicationRequestSubstitution>,
    #[fhir_serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

/// Choice of types for the allowed\[x\] field in MedicationRequestSubstitution
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "allowed")]
pub enum MedicationRequestSubstitutionAllowed {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "allowedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "allowedCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "allowed")]
pub struct MedicationRequestSubstitution {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub allowed: Option<MedicationRequestSubstitutionAllowed>,
    pub reason: Option<CodeableConcept>,
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
    pub dispenser: Option<Reference>,
    #[fhir_serde(rename = "dispenserInstruction")]
    pub dispenser_instruction: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "doseAdministrationAid")]
    pub dose_administration_aid: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicationStatementAdherence {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub reason: Option<CodeableConcept>,
}

/// Choice of types for the effective\[x\] field in MedicationStatement
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "effective")]
pub enum MedicationStatementEffective {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "effectivePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "effectiveTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "effective")]
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
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub medication: CodeableReference,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub effective: Option<MedicationStatementEffective>,
    #[fhir_serde(rename = "dateAsserted")]
    pub date_asserted: Option<DateTime>,
    pub author: Option<Reference>,
    #[fhir_serde(rename = "informationSource")]
    pub information_source: Option<Vec<Reference>>,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    pub reason: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "relatedClinicalInformation")]
    pub related_clinical_information: Option<Vec<Reference>>,
    #[fhir_serde(rename = "renderedDosageInstruction")]
    pub rendered_dosage_instruction: Option<Markdown>,
    pub dosage: Option<Vec<Dosage>>,
    pub adherence: Option<MedicationStatementAdherence>,
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
pub struct MedicinalProductDefinitionNameUsage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    pub jurisdiction: Option<CodeableConcept>,
    pub language: CodeableConcept,
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
    #[fhir_serde(rename = "comprisedOf")]
    pub comprised_of: Option<Vec<Reference>>,
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

/// Choice of types for the value\[x\] field in MedicinalProductDefinitionCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum MedicinalProductDefinitionCharacteristicValue {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "valueInteger")]
    Integer(Integer),
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
#[fhir_resource(choice_elements = "value")]
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
    pub part: Option<Vec<MedicinalProductDefinitionNamePart>>,
    pub usage: Option<Vec<MedicinalProductDefinitionNameUsage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MedicinalProductDefinitionNamePart {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub part: String,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
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

/// Choice of types for the versionAlgorithm\[x\] field in MessageDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum MessageDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

/// Choice of types for the event\[x\] field in MessageDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "event")]
pub enum MessageDefinitionEvent {
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm,event")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<MessageDefinitionVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
    pub graph: Option<Canonical>,
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

/// Choice of types for the endpoint\[x\] field in MessageHeaderDestination
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "endpoint")]
pub enum MessageHeaderDestinationEndpoint {
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "endpointUrl")]
    Url(Url),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "endpointReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "endpoint")]
pub struct MessageHeaderDestination {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub endpoint: Option<MessageHeaderDestinationEndpoint>,
    pub name: Option<String>,
    pub receiver: Option<Reference>,
}

/// Choice of types for the endpoint\[x\] field in MessageHeaderSource
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "endpoint")]
pub enum MessageHeaderSourceEndpoint {
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "endpointUrl")]
    Url(Url),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "endpointReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "endpoint")]
pub struct MessageHeaderSource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub endpoint: Option<MessageHeaderSourceEndpoint>,
    pub name: Option<String>,
    pub software: Option<String>,
    pub version: Option<String>,
    pub contact: Option<ContactPoint>,
    pub sender: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MessageHeaderResponse {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Identifier,
    pub code: Code,
    pub details: Option<Reference>,
}

/// Choice of types for the event\[x\] field in MessageHeader
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "event")]
pub enum MessageHeaderEvent {
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "eventUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "eventCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "event")]
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
    pub source: MessageHeaderSource,
    pub reason: Option<CodeableConcept>,
    pub response: Option<MessageHeaderResponse>,
    pub focus: Option<Vec<Reference>>,
    pub definition: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionLocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "sequenceLocation")]
    pub sequence_location: Option<MolecularDefinitionLocationSequenceLocation>,
    #[fhir_serde(rename = "cytobandLocation")]
    pub cytoband_location: Option<MolecularDefinitionLocationCytobandLocation>,
}

/// Choice of types for the description\[x\] field in MolecularDefinitionLocationCytobandLocationGenomeAssembly
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "description")]
pub enum MolecularDefinitionLocationCytobandLocationGenomeAssemblyDescription {
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "descriptionMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "descriptionString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "description")]
pub struct MolecularDefinitionLocationCytobandLocationGenomeAssembly {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub organism: Option<CodeableConcept>,
    pub build: Option<CodeableConcept>,
    pub accession: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub description: Option<MolecularDefinitionLocationCytobandLocationGenomeAssemblyDescription>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionLocationCytobandLocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "genomeAssembly")]
    pub genome_assembly: MolecularDefinitionLocationCytobandLocationGenomeAssembly,
    #[fhir_serde(rename = "cytobandInterval")]
    pub cytoband_interval: MolecularDefinitionLocationCytobandLocationCytobandInterval,
}

/// Choice of types for the start\[x\] field in MolecularDefinitionRepresentationExtractedCoordinateInterval
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "start")]
pub enum MolecularDefinitionRepresentationExtractedCoordinateIntervalStart {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "startQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "startRange")]
    Range(Range),
}

/// Choice of types for the end\[x\] field in MolecularDefinitionRepresentationExtractedCoordinateInterval
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "end")]
pub enum MolecularDefinitionRepresentationExtractedCoordinateIntervalEnd {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "endQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "endRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "start,end")]
pub struct MolecularDefinitionRepresentationExtractedCoordinateInterval {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "coordinateSystem")]
    pub coordinate_system:
        Option<MolecularDefinitionRepresentationExtractedCoordinateIntervalCoordinateSystem>,
    #[fhir_serde(flatten)]
    pub start: Option<MolecularDefinitionRepresentationExtractedCoordinateIntervalStart>,
    #[fhir_serde(flatten)]
    pub end: Option<MolecularDefinitionRepresentationExtractedCoordinateIntervalEnd>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationExtractedCoordinateIntervalCoordinateSystem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<CodeableConcept>,
    pub origin: Option<CodeableConcept>,
    #[fhir_serde(rename = "normalizationMethod")]
    pub normalization_method: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationRelative {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "startingMolecule")]
    pub starting_molecule: Reference,
    pub edit: Option<Vec<MolecularDefinitionRepresentationRelativeEdit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinition {
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
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "moleculeType")]
    pub molecule_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub topology: Option<Vec<CodeableConcept>>,
    pub member: Option<Vec<Reference>>,
    pub location: Option<Vec<MolecularDefinitionLocation>>,
    pub representation: Option<Vec<MolecularDefinitionRepresentation>>,
}

/// Choice of types for the start\[x\] field in MolecularDefinitionLocationSequenceLocationCoordinateInterval
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "start")]
pub enum MolecularDefinitionLocationSequenceLocationCoordinateIntervalStart {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "startQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "startRange")]
    Range(Range),
}

/// Choice of types for the end\[x\] field in MolecularDefinitionLocationSequenceLocationCoordinateInterval
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "end")]
pub enum MolecularDefinitionLocationSequenceLocationCoordinateIntervalEnd {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "endQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "endRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "start,end")]
pub struct MolecularDefinitionLocationSequenceLocationCoordinateInterval {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "coordinateSystem")]
    pub coordinate_system:
        Option<MolecularDefinitionLocationSequenceLocationCoordinateIntervalCoordinateSystem>,
    #[fhir_serde(flatten)]
    pub start: Option<MolecularDefinitionLocationSequenceLocationCoordinateIntervalStart>,
    #[fhir_serde(flatten)]
    pub end: Option<MolecularDefinitionLocationSequenceLocationCoordinateIntervalEnd>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionLocationSequenceLocationCoordinateIntervalCoordinateSystem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<CodeableConcept>,
    pub origin: Option<CodeableConcept>,
    #[fhir_serde(rename = "normalizationMethod")]
    pub normalization_method: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationConcatenated {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "sequenceElement")]
    pub sequence_element: Option<Vec<MolecularDefinitionRepresentationConcatenatedSequenceElement>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionLocationSequenceLocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "sequenceContext")]
    pub sequence_context: Reference,
    #[fhir_serde(rename = "coordinateInterval")]
    pub coordinate_interval: Option<MolecularDefinitionLocationSequenceLocationCoordinateInterval>,
    pub strand: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub focus: Option<CodeableConcept>,
    pub code: Option<Vec<CodeableConcept>>,
    pub literal: Option<MolecularDefinitionRepresentationLiteral>,
    pub resolvable: Option<Reference>,
    pub extracted: Option<MolecularDefinitionRepresentationExtracted>,
    pub repeated: Option<MolecularDefinitionRepresentationRepeated>,
    pub concatenated: Option<MolecularDefinitionRepresentationConcatenated>,
    pub relative: Option<MolecularDefinitionRepresentationRelative>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationConcatenatedSequenceElement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Reference,
    #[fhir_serde(rename = "ordinalIndex")]
    pub ordinal_index: Integer,
}

/// Choice of types for the arm\[x\] field in MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytoband
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "arm")]
pub enum MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytobandArm {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "armCode")]
    Code(Code),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "armString")]
    String(String),
}

/// Choice of types for the region\[x\] field in MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytoband
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "region")]
pub enum MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytobandRegion {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "regionCode")]
    Code(Code),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "regionString")]
    String(String),
}

/// Choice of types for the band\[x\] field in MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytoband
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "band")]
pub enum MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytobandBand {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "bandCode")]
    Code(Code),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "bandString")]
    String(String),
}

/// Choice of types for the subBand\[x\] field in MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytoband
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subBand")]
pub enum MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytobandSubBand {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "subBandCode")]
    Code(Code),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "subBandString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "arm,region,band,subBand")]
pub struct MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytoband {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub arm: Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytobandArm>,
    #[fhir_serde(flatten)]
    pub region:
        Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytobandRegion>,
    #[fhir_serde(flatten)]
    pub band: Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytobandBand>,
    #[fhir_serde(flatten)]
    pub sub_band:
        Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytobandSubBand>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationRepeated {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "sequenceMotif")]
    pub sequence_motif: Reference,
    #[fhir_serde(rename = "copyCount")]
    pub copy_count: Integer,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationRelativeEdit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "editOrder")]
    pub edit_order: Option<Integer>,
    #[fhir_serde(rename = "coordinateInterval")]
    pub coordinate_interval:
        Option<MolecularDefinitionRepresentationRelativeEditCoordinateInterval>,
    #[fhir_serde(rename = "replacementMolecule")]
    pub replacement_molecule: Reference,
    #[fhir_serde(rename = "replacedMolecule")]
    pub replaced_molecule: Option<Reference>,
}

/// Choice of types for the start\[x\] field in MolecularDefinitionRepresentationRelativeEditCoordinateInterval
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "start")]
pub enum MolecularDefinitionRepresentationRelativeEditCoordinateIntervalStart {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "startQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "startRange")]
    Range(Range),
}

/// Choice of types for the end\[x\] field in MolecularDefinitionRepresentationRelativeEditCoordinateInterval
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "end")]
pub enum MolecularDefinitionRepresentationRelativeEditCoordinateIntervalEnd {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "endQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "endRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "start,end")]
pub struct MolecularDefinitionRepresentationRelativeEditCoordinateInterval {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "coordinateSystem")]
    pub coordinate_system:
        Option<MolecularDefinitionRepresentationRelativeEditCoordinateIntervalCoordinateSystem>,
    #[fhir_serde(flatten)]
    pub start: Option<MolecularDefinitionRepresentationRelativeEditCoordinateIntervalStart>,
    #[fhir_serde(flatten)]
    pub end: Option<MolecularDefinitionRepresentationRelativeEditCoordinateIntervalEnd>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationExtracted {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "startingMolecule")]
    pub starting_molecule: Reference,
    #[fhir_serde(rename = "coordinateInterval")]
    pub coordinate_interval: Option<MolecularDefinitionRepresentationExtractedCoordinateInterval>,
    #[fhir_serde(rename = "reverseComplement")]
    pub reverse_complement: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationRelativeEditCoordinateIntervalCoordinateSystem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<CodeableConcept>,
    pub origin: Option<CodeableConcept>,
    #[fhir_serde(rename = "normalizationMethod")]
    pub normalization_method: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionLocationCytobandLocationCytobandInterval {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub chromosome: CodeableConcept,
    #[fhir_serde(rename = "startCytoband")]
    pub start_cytoband:
        Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytoband>,
    #[fhir_serde(rename = "endCytoband")]
    pub end_cytoband:
        Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalEndCytoband>,
}

/// Choice of types for the arm\[x\] field in MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytoband
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "arm")]
pub enum MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytobandArm {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "armCode")]
    Code(Code),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "armString")]
    String(String),
}

/// Choice of types for the region\[x\] field in MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytoband
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "region")]
pub enum MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytobandRegion {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "regionCode")]
    Code(Code),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "regionString")]
    String(String),
}

/// Choice of types for the band\[x\] field in MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytoband
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "band")]
pub enum MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytobandBand {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "bandCode")]
    Code(Code),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "bandString")]
    String(String),
}

/// Choice of types for the subBand\[x\] field in MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytoband
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subBand")]
pub enum MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytobandSubBand {
    /// Variant accepting the Code type.
    #[fhir_serde(rename = "subBandCode")]
    Code(Code),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "subBandString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "arm,region,band,subBand")]
pub struct MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytoband {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub arm: Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytobandArm>,
    #[fhir_serde(flatten)]
    pub region:
        Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytobandRegion>,
    #[fhir_serde(flatten)]
    pub band: Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytobandBand>,
    #[fhir_serde(flatten)]
    pub sub_band:
        Option<MolecularDefinitionLocationCytobandLocationCytobandIntervalStartCytobandSubBand>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct MolecularDefinitionRepresentationLiteral {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub encoding: Option<CodeableConcept>,
    pub value: String,
}

/// Choice of types for the versionAlgorithm\[x\] field in NamingSystem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum NamingSystemVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<NamingSystemVersionAlgorithm>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub kind: Code,
    pub experimental: Option<Boolean>,
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
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
    pub usage: Option<String>,
    #[fhir_serde(rename = "uniqueId")]
    pub unique_id: Option<Vec<NamingSystemUniqueId>>,
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
    pub authoritative: Option<Boolean>,
}

/// Choice of types for the occurrence\[x\] field in NutritionIntake
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
pub enum NutritionIntakeOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
}

/// Choice of types for the reported\[x\] field in NutritionIntake
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "reported")]
pub enum NutritionIntakeReported {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "reportedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "reportedReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "occurrence,reported")]
pub struct NutritionIntake {
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
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<NutritionIntakeOccurrence>,
    pub recorded: Option<DateTime>,
    #[fhir_serde(flatten)]
    pub reported: Option<NutritionIntakeReported>,
    #[fhir_serde(rename = "nutritionItem")]
    pub nutrition_item: Option<Vec<NutritionIntakeNutritionItem>>,
    pub performer: Option<Vec<NutritionIntakePerformer>>,
    pub location: Option<Reference>,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    pub reason: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionIntakeNutritionItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "nutritionProduct")]
    pub nutrition_product: Option<CodeableReference>,
    #[fhir_serde(rename = "consumedItem")]
    pub consumed_item: Option<Vec<NutritionIntakeNutritionItemConsumedItem>>,
    #[fhir_serde(rename = "notConsumedItem")]
    pub not_consumed_item: Option<Vec<NutritionIntakeNutritionItemNotConsumedItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionIntakePerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

/// Choice of types for the rate\[x\] field in NutritionIntakeNutritionItemConsumedItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "rate")]
pub enum NutritionIntakeNutritionItemConsumedItemRate {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "rateQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "rateRatio")]
    Ratio(Ratio),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "rate")]
pub struct NutritionIntakeNutritionItemConsumedItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: Option<Timing>,
    pub amount: Option<Quantity>,
    #[fhir_serde(flatten)]
    pub rate: Option<NutritionIntakeNutritionItemConsumedItemRate>,
    #[fhir_serde(rename = "totalIntake")]
    pub total_intake: Option<Vec<NutritionIntakeNutritionItemConsumedItemTotalIntake>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionIntakeNutritionItemConsumedItemTotalIntake {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub nutrient: CodeableReference,
    pub amount: Quantity,
    pub energy: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionIntakeNutritionItemNotConsumedItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reason: Option<CodeableConcept>,
    pub schedule: Option<Timing>,
    pub amount: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderSupplement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableReference>,
    #[fhir_serde(rename = "productName")]
    pub product_name: Option<String>,
    pub schedule: Option<NutritionOrderSupplementSchedule>,
    pub quantity: Option<Quantity>,
    pub instruction: Option<String>,
    #[fhir_serde(rename = "caloricDensity")]
    pub caloric_density: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderEnteralFormula {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableReference>,
    #[fhir_serde(rename = "productName")]
    pub product_name: Option<String>,
    #[fhir_serde(rename = "deliveryDevice")]
    pub delivery_device: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "caloricDensity")]
    pub caloric_density: Option<Quantity>,
    #[fhir_serde(rename = "routeOfAdministration")]
    pub route_of_administration: Option<Vec<CodeableConcept>>,
    pub administration: Option<Vec<NutritionOrderEnteralFormulaAdministration>>,
    #[fhir_serde(rename = "maxVolumeToAdminister")]
    pub max_volume_to_administer: Option<Quantity>,
    #[fhir_serde(rename = "administrationInstruction")]
    pub administration_instruction: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderAdditive {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modularType")]
    pub modular_type: Option<CodeableReference>,
    #[fhir_serde(rename = "productName")]
    pub product_name: Option<String>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(rename = "routeOfAdministration")]
    pub route_of_administration: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderSupplementSchedule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub timing: Option<Vec<Timing>>,
    #[fhir_serde(rename = "asNeeded")]
    pub as_needed: Option<Boolean>,
    #[fhir_serde(rename = "asNeededFor")]
    pub as_needed_for: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderOralDietSchedule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub timing: Option<Vec<Timing>>,
    #[fhir_serde(rename = "asNeeded")]
    pub as_needed: Option<Boolean>,
    #[fhir_serde(rename = "asNeededFor")]
    pub as_needed_for: Option<CodeableConcept>,
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
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    pub priority: Option<Code>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[fhir_serde(rename = "dateTime")]
    pub date_time: DateTime,
    pub requester: Option<Reference>,
    pub performer: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "allergyIntolerance")]
    pub allergy_intolerance: Option<Vec<Reference>>,
    #[fhir_serde(rename = "foodPreferenceModifier")]
    pub food_preference_modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "excludeFoodModifier")]
    pub exclude_food_modifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "outsideFoodAllowed")]
    pub outside_food_allowed: Option<Boolean>,
    #[fhir_serde(rename = "oralDiet")]
    pub oral_diet: Option<NutritionOrderOralDiet>,
    pub supplement: Option<Vec<NutritionOrderSupplement>>,
    #[fhir_serde(rename = "enteralFormula")]
    pub enteral_formula: Option<NutritionOrderEnteralFormula>,
    pub additive: Option<Vec<NutritionOrderAdditive>>,
    pub note: Option<Vec<Annotation>>,
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
pub struct NutritionOrderOralDietTexture {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub modifier: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderOralDiet {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub schedule: Option<NutritionOrderOralDietSchedule>,
    pub nutrient: Option<Vec<NutritionOrderOralDietNutrient>>,
    pub texture: Option<Vec<NutritionOrderOralDietTexture>>,
    pub instruction: Option<String>,
    #[fhir_serde(rename = "caloricDensity")]
    pub caloric_density: Option<Quantity>,
}

/// Choice of types for the rate\[x\] field in NutritionOrderEnteralFormulaAdministration
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "rate")]
pub enum NutritionOrderEnteralFormulaAdministrationRate {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "rateQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "rateRatio")]
    Ratio(Ratio),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "rate")]
pub struct NutritionOrderEnteralFormulaAdministration {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: Option<NutritionOrderEnteralFormulaAdministrationSchedule>,
    pub quantity: Option<Quantity>,
    #[fhir_serde(flatten)]
    pub rate: Option<NutritionOrderEnteralFormulaAdministrationRate>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionOrderEnteralFormulaAdministrationSchedule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub timing: Option<Vec<Timing>>,
    #[fhir_serde(rename = "asNeeded")]
    pub as_needed: Option<Boolean>,
    #[fhir_serde(rename = "asNeededFor")]
    pub as_needed_for: Option<CodeableConcept>,
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
    pub code: Option<CodeableConcept>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub manufacturer: Option<Vec<Reference>>,
    pub nutrient: Option<Vec<NutritionProductNutrient>>,
    #[fhir_serde(rename = "ingredientSummary")]
    pub ingredient_summary: Option<Markdown>,
    pub ingredient: Option<Vec<NutritionProductIngredient>>,
    pub energy: Option<Quantity>,
    pub characteristic: Option<Vec<NutritionProductCharacteristic>>,
    pub instance: Option<Vec<NutritionProductInstance>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct NutritionProductInstance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    pub identifier: Option<Vec<Identifier>>,
    pub name: Option<String>,
    #[fhir_serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    pub expiry: Option<DateTime>,
    #[fhir_serde(rename = "useBy")]
    pub use_by: Option<DateTime>,
    #[fhir_serde(rename = "biologicalSourceEvent")]
    pub biological_source_event: Option<Identifier>,
}

/// Choice of types for the value\[x\] field in NutritionProductCharacteristic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum NutritionProductCharacteristicValue {
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
#[fhir_resource(choice_elements = "value")]
pub struct NutritionProductCharacteristic {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<NutritionProductCharacteristicValue>,
}

/// Choice of types for the amount\[x\] field in NutritionProductNutrient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "amount")]
pub enum NutritionProductNutrientAmount {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "amountRatio")]
    Ratio(Ratio),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "amountQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "amount")]
pub struct NutritionProductNutrient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    #[fhir_serde(flatten)]
    pub amount: Option<NutritionProductNutrientAmount>,
}

/// Choice of types for the amount\[x\] field in NutritionProductIngredient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "amount")]
pub enum NutritionProductIngredientAmount {
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "amountRatio")]
    Ratio(Ratio),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "amountQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "amount")]
pub struct NutritionProductIngredient {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    #[fhir_serde(flatten)]
    pub amount: Option<NutritionProductIngredientAmount>,
    pub allergen: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ObservationReferenceRange {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub low: Option<Quantity>,
    pub high: Option<Quantity>,
    #[fhir_serde(rename = "normalValue")]
    pub normal_value: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "appliesTo")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    pub age: Option<Range>,
    pub text: Option<Markdown>,
}

/// Choice of types for the instantiates\[x\] field in Observation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "instantiates")]
pub enum ObservationInstantiates {
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "instantiatesCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "instantiatesReference")]
    Reference(Reference),
}

/// Choice of types for the effective\[x\] field in Observation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "effective")]
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
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "instantiates,effective,value")]
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
    #[fhir_serde(flatten)]
    pub instantiates: Option<ObservationInstantiates>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "triggeredBy")]
    pub triggered_by: Option<Vec<ObservationTriggeredBy>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub focus: Option<Vec<Reference>>,
    pub organizer: Option<Boolean>,
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
    #[fhir_serde(rename = "bodyStructure")]
    pub body_structure: Option<Reference>,
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
pub struct ObservationTriggeredBy {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub observation: Reference,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub reason: Option<String>,
}

/// Choice of types for the value\[x\] field in ObservationComponent
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ObservationDefinitionComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(rename = "permittedDataType")]
    pub permitted_data_type: Option<Vec<Code>>,
    #[fhir_serde(rename = "permittedUnit")]
    pub permitted_unit: Option<Vec<Coding>>,
    #[fhir_serde(rename = "qualifiedValue")]
    pub qualified_value: Option<Vec<ObservationDefinitionQualifiedValue>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in ObservationDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ObservationDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub url: Option<Uri>,
    pub identifier: Option<Identifier>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<ObservationDefinitionVersionAlgorithm>,
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
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[fhir_serde(rename = "derivedFromCanonical")]
    pub derived_from_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "derivedFromUri")]
    pub derived_from_uri: Option<Vec<Uri>>,
    pub subject: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    #[fhir_serde(rename = "permittedDataType")]
    pub permitted_data_type: Option<Vec<Code>>,
    #[fhir_serde(rename = "multipleResultsAllowed")]
    pub multiple_results_allowed: Option<Boolean>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub specimen: Option<Vec<Reference>>,
    pub device: Option<Vec<Reference>>,
    #[fhir_serde(rename = "preferredReportName")]
    pub preferred_report_name: Option<String>,
    #[fhir_serde(rename = "permittedUnit")]
    pub permitted_unit: Option<Vec<Coding>>,
    #[fhir_serde(rename = "qualifiedValue")]
    pub qualified_value: Option<Vec<ObservationDefinitionQualifiedValue>>,
    #[fhir_serde(rename = "hasMember")]
    pub has_member: Option<Vec<Reference>>,
    pub component: Option<Vec<ObservationDefinitionComponent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ObservationDefinitionQualifiedValue {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Option<CodeableConcept>,
    #[fhir_serde(rename = "appliesTo")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "sexParameterForClinicalUse")]
    pub sex_parameter_for_clinical_use: Option<Code>,
    pub age: Option<Range>,
    #[fhir_serde(rename = "gestationalAge")]
    pub gestational_age: Option<Range>,
    pub condition: Option<String>,
    #[fhir_serde(rename = "rangeCategory")]
    pub range_category: Option<Code>,
    pub range: Option<Range>,
    #[fhir_serde(rename = "validCodedValueSet")]
    pub valid_coded_value_set: Option<Canonical>,
    #[fhir_serde(rename = "normalCodedValueSet")]
    pub normal_coded_value_set: Option<Canonical>,
    #[fhir_serde(rename = "abnormalCodedValueSet")]
    pub abnormal_coded_value_set: Option<Canonical>,
    #[fhir_serde(rename = "criticalCodedValueSet")]
    pub critical_coded_value_set: Option<Canonical>,
    pub interpretation: Option<Vec<CodeableConcept>>,
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

/// Choice of types for the versionAlgorithm\[x\] field in OperationDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum OperationDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<OperationDefinitionVersionAlgorithm>,
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
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    #[fhir_serde(rename = "affectsState")]
    pub affects_state: Option<Boolean>,
    pub synchronicity: Option<Code>,
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
pub struct OperationDefinitionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    #[fhir_serde(rename = "use")]
    pub r#use: Code,
    pub scope: Option<Vec<Code>>,
    pub min: UnsignedInt,
    pub max: String,
    pub documentation: Option<Markdown>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    #[fhir_serde(rename = "allowedType")]
    pub allowed_type: Option<Vec<Code>>,
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
    pub description: Option<Markdown>,
    pub contact: Option<Vec<ExtendedContactDetail>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Reference>,
    pub endpoint: Option<Vec<Reference>>,
    pub qualification: Option<Vec<OrganizationQualification>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct OrganizationQualification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    pub status: Option<CodeableConcept>,
    pub period: Option<Period>,
    pub issuer: Option<Reference>,
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
    pub contact: Option<Vec<ExtendedContactDetail>>,
    pub endpoint: Option<Vec<Reference>>,
}

/// Choice of types for the value\[x\] field in PackagedProductDefinitionPackagingProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum PackagedProductDefinitionPackagingPropertyValue {
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
#[fhir_resource(choice_elements = "value")]
pub struct PackagedProductDefinitionPackagingProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<PackagedProductDefinitionPackagingPropertyValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PackagedProductDefinitionPackagingContainedItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    pub amount: Option<Quantity>,
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
pub struct PackagedProductDefinitionPackaging {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(rename = "componentPart")]
    pub component_part: Option<Boolean>,
    pub quantity: Option<Integer>,
    pub material: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "alternateMaterial")]
    pub alternate_material: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "shelfLifeStorage")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    pub manufacturer: Option<Vec<Reference>>,
    pub property: Option<Vec<PackagedProductDefinitionPackagingProperty>>,
    #[fhir_serde(rename = "containedItem")]
    pub contained_item: Option<Vec<PackagedProductDefinitionPackagingContainedItem>>,
    pub packaging: Option<Vec<PackagedProductDefinitionPackaging>>,
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
    #[fhir_serde(rename = "copackagedIndicator")]
    pub copackaged_indicator: Option<Boolean>,
    pub manufacturer: Option<Vec<Reference>>,
    #[fhir_serde(rename = "attachedDocument")]
    pub attached_document: Option<Vec<Reference>>,
    pub packaging: Option<PackagedProductDefinitionPackaging>,
    pub characteristic: Option<Vec<PackagedProductDefinitionPackagingProperty>>,
}

/// Choice of types for the value\[x\] field in ParametersParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "valueInteger64")]
    Integer64(Integer64),
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
    /// Variant accepting the Availability type.
    #[fhir_serde(rename = "valueAvailability")]
    Availability(Availability),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "valueExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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
pub struct PatientContact {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationship: Option<Vec<CodeableConcept>>,
    pub role: Option<Vec<CodeableConcept>>,
    pub name: Option<HumanName>,
    #[fhir_serde(rename = "additionalName")]
    pub additional_name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
    #[fhir_serde(rename = "additionalAddress")]
    pub additional_address: Option<Vec<Address>>,
    pub gender: Option<Code>,
    pub organization: Option<Reference>,
    pub period: Option<Period>,
}

/// Choice of types for the deceased\[x\] field in Patient
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "deceased")]
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
#[fhir_choice_element(base_name = "multipleBirth")]
pub enum PatientMultipleBirth {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "multipleBirthBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[fhir_serde(rename = "multipleBirthInteger")]
    Integer(Integer),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "deceased,multipleBirth")]
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
pub struct PatientCommunication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
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
    pub reporter: Option<Reference>,
    pub payment: Option<Reference>,
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

/// Choice of types for the targetItem\[x\] field in PaymentReconciliationAllocation
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "targetItem")]
pub enum PaymentReconciliationAllocationTargetItem {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "targetItemString")]
    String(String),
    /// Variant accepting the Identifier type.
    #[fhir_serde(rename = "targetItemIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the PositiveInt type.
    #[fhir_serde(rename = "targetItemPositiveInt")]
    PositiveInt(PositiveInt),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "targetItem")]
pub struct PaymentReconciliationAllocation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub predecessor: Option<Identifier>,
    pub target: Option<Reference>,
    #[fhir_serde(flatten)]
    pub target_item: Option<PaymentReconciliationAllocationTargetItem>,
    pub encounter: Option<Reference>,
    pub account: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub submitter: Option<Reference>,
    pub response: Option<Reference>,
    pub date: Option<Date>,
    pub responsible: Option<Reference>,
    pub payee: Option<Reference>,
    pub amount: Option<Money>,
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
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub status: Code,
    pub kind: Option<CodeableConcept>,
    pub period: Option<Period>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    #[fhir_serde(rename = "issuerType")]
    pub issuer_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "paymentIssuer")]
    pub payment_issuer: Option<Reference>,
    pub request: Option<Reference>,
    pub requestor: Option<Reference>,
    pub outcome: Option<Code>,
    pub disposition: Option<String>,
    pub date: Date,
    pub location: Option<Reference>,
    pub method: Option<CodeableConcept>,
    #[fhir_serde(rename = "cardBrand")]
    pub card_brand: Option<String>,
    #[fhir_serde(rename = "accountNumber")]
    pub account_number: Option<String>,
    #[fhir_serde(rename = "expirationDate")]
    pub expiration_date: Option<Date>,
    pub processor: Option<String>,
    #[fhir_serde(rename = "referenceNumber")]
    pub reference_number: Option<String>,
    pub authorization: Option<String>,
    #[fhir_serde(rename = "tenderedAmount")]
    pub tendered_amount: Option<Money>,
    #[fhir_serde(rename = "returnedAmount")]
    pub returned_amount: Option<Money>,
    pub amount: Money,
    #[fhir_serde(rename = "paymentIdentifier")]
    pub payment_identifier: Option<Identifier>,
    pub allocation: Option<Vec<PaymentReconciliationAllocation>>,
    #[fhir_serde(rename = "formCode")]
    pub form_code: Option<CodeableConcept>,
    #[fhir_serde(rename = "processNote")]
    pub process_note: Option<Vec<PaymentReconciliationProcessNote>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PermissionRuleActivityActor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub reference: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Permission {
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
    pub asserter: Option<Reference>,
    pub date: Option<Vec<DateTime>>,
    pub validity: Option<Period>,
    pub justification: Option<PermissionJustification>,
    pub combining: Code,
    pub rule: Option<Vec<PermissionRule>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PermissionJustification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub basis: Option<Vec<CodeableConcept>>,
    pub evidence: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PermissionRuleDataResource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub meaning: Code,
    pub reference: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PermissionRule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub import: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    pub data: Option<Vec<PermissionRuleData>>,
    pub activity: Option<Vec<PermissionRuleActivity>>,
    pub limit: Option<Vec<PermissionRuleLimit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PermissionRuleData {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub resource: Option<Vec<PermissionRuleDataResource>>,
    #[fhir_serde(rename = "resourceType")]
    pub resource_type: Option<Vec<Coding>>,
    pub security: Option<Vec<Coding>>,
    pub period: Option<Period>,
    pub expression: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PermissionRuleLimit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub control: Option<Vec<CodeableConcept>>,
    pub tag: Option<Vec<Coding>>,
    pub element: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PermissionRuleActivity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub actor: Option<Vec<PermissionRuleActivityActor>>,
    pub action: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PersonCommunication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
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

/// Choice of types for the deceased\[x\] field in Person
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "deceased")]
pub enum PersonDeceased {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "deceasedDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "deceased")]
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
    pub active: Option<Boolean>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    #[fhir_serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    #[fhir_serde(flatten)]
    pub deceased: Option<PersonDeceased>,
    pub address: Option<Vec<Address>>,
    #[fhir_serde(rename = "maritalStatus")]
    pub marital_status: Option<CodeableConcept>,
    pub photo: Option<Vec<Attachment>>,
    pub communication: Option<Vec<PersonCommunication>>,
    #[fhir_serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub link: Option<Vec<PersonLink>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PersonalRelationship {
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
    pub source: Reference,
    #[fhir_serde(rename = "relationshipType")]
    pub relationship_type: CodeableConcept,
    pub target: Reference,
    pub period: Option<Vec<Period>>,
    pub confidence: Option<CodeableConcept>,
    pub asserter: Option<Reference>,
    pub group: Option<Vec<Reference>>,
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

/// Choice of types for the versionAlgorithm\[x\] field in PlanDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum PlanDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

/// Choice of types for the subject\[x\] field in PlanDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
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

/// Choice of types for the asNeeded\[x\] field in PlanDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "asNeeded")]
pub enum PlanDefinitionAsNeeded {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm,subject,asNeeded")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<PlanDefinitionVersionAlgorithm>,
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
    pub usage: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
    pub actor: Option<Vec<PlanDefinitionActor>>,
    pub action: Option<Vec<PlanDefinitionAction>>,
    #[fhir_serde(flatten)]
    pub as_needed: Option<PlanDefinitionAsNeeded>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionActionInput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub requirement: Option<DataRequirement>,
    #[fhir_serde(rename = "relatedData")]
    pub related_data: Option<String>,
}

/// Choice of types for the subject\[x\] field in PlanDefinitionAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
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
#[fhir_choice_element(base_name = "timing")]
pub enum PlanDefinitionActionTiming {
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "timingAge")]
    Age(Age),
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
#[fhir_choice_element(base_name = "definition")]
pub enum PlanDefinitionActionDefinition {
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "definitionUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "subject,timing,definition")]
pub struct PlanDefinitionAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub prefix: Option<String>,
    pub title: Option<String>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "textEquivalent")]
    pub text_equivalent: Option<Markdown>,
    pub priority: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[fhir_serde(rename = "goalId")]
    pub goal_id: Option<Vec<String>>,
    #[fhir_serde(flatten)]
    pub subject: Option<PlanDefinitionActionSubject>,
    pub trigger: Option<Vec<TriggerDefinition>>,
    pub condition: Option<Vec<PlanDefinitionActionCondition>>,
    pub input: Option<Vec<PlanDefinitionActionInput>>,
    pub output: Option<Vec<PlanDefinitionActionOutput>>,
    #[fhir_serde(rename = "relatedAction")]
    pub related_action: Option<Vec<PlanDefinitionActionRelatedAction>>,
    #[fhir_serde(flatten)]
    pub timing: Option<PlanDefinitionActionTiming>,
    pub location: Option<CodeableReference>,
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
pub struct PlanDefinitionActor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub description: Option<Markdown>,
    pub option: Option<Vec<PlanDefinitionActorOption>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionActionOutput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub requirement: Option<DataRequirement>,
    #[fhir_serde(rename = "relatedData")]
    pub related_data: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PlanDefinitionActionParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "actorId")]
    pub actor_id: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    #[fhir_serde(rename = "typeCanonical")]
    pub type_canonical: Option<Canonical>,
    #[fhir_serde(rename = "typeReference")]
    pub type_reference: Option<Reference>,
    pub role: Option<CodeableConcept>,
    pub function: Option<CodeableConcept>,
}

/// Choice of types for the offset\[x\] field in PlanDefinitionActionRelatedAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "offset")]
pub enum PlanDefinitionActionRelatedActionOffset {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "offset")]
pub struct PlanDefinitionActionRelatedAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "targetId")]
    pub target_id: String,
    pub relationship: Code,
    #[fhir_serde(rename = "endRelationship")]
    pub end_relationship: Option<Code>,
    #[fhir_serde(flatten)]
    pub offset: Option<PlanDefinitionActionRelatedActionOffset>,
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

/// Choice of types for the detail\[x\] field in PlanDefinitionGoalTarget
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "detail")]
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "detail")]
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
pub struct PlanDefinitionActorOption {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    #[fhir_serde(rename = "typeCanonical")]
    pub type_canonical: Option<Canonical>,
    #[fhir_serde(rename = "typeReference")]
    pub type_reference: Option<Reference>,
    pub role: Option<CodeableConcept>,
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

/// Choice of types for the deceased\[x\] field in Practitioner
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "deceased")]
pub enum PractitionerDeceased {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "deceasedDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "deceased")]
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
    pub gender: Option<Code>,
    #[fhir_serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    #[fhir_serde(flatten)]
    pub deceased: Option<PractitionerDeceased>,
    pub address: Option<Vec<Address>>,
    pub photo: Option<Vec<Attachment>>,
    pub qualification: Option<Vec<PractitionerQualification>>,
    pub communication: Option<Vec<PractitionerCommunication>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PractitionerQualification {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    pub status: Option<CodeableConcept>,
    pub period: Option<Period>,
    pub issuer: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct PractitionerCommunication {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
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
    pub network: Option<Vec<Reference>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub display: Option<String>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    #[fhir_serde(rename = "healthcareService")]
    pub healthcare_service: Option<Vec<Reference>>,
    pub contact: Option<Vec<ExtendedContactDetail>>,
    pub characteristic: Option<Vec<CodeableConcept>>,
    pub communication: Option<Vec<CodeableConcept>>,
    pub availability: Option<Availability>,
    pub endpoint: Option<Vec<Reference>>,
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
    pub period: Option<Period>,
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

/// Choice of types for the occurrence\[x\] field in Procedure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
pub enum ProcedureOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "occurrenceString")]
    String(String),
    /// Variant accepting the Age type.
    #[fhir_serde(rename = "occurrenceAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "occurrenceRange")]
    Range(Range),
    /// Variant accepting the Timing type.
    #[fhir_serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

/// Choice of types for the reported\[x\] field in Procedure
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "reported")]
pub enum ProcedureReported {
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "reportedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "reportedReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "occurrence,reported")]
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
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub focus: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<ProcedureOccurrence>,
    pub recorded: Option<DateTime>,
    pub recorder: Option<Reference>,
    #[fhir_serde(flatten)]
    pub reported: Option<ProcedureReported>,
    pub performer: Option<Vec<ProcedurePerformer>>,
    pub location: Option<Reference>,
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "bodyStructure")]
    pub body_structure: Option<Reference>,
    pub outcome: Option<Vec<CodeableReference>>,
    pub report: Option<Vec<Reference>>,
    pub complication: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "followUp")]
    pub follow_up: Option<Vec<CodeableReference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "focalDevice")]
    pub focal_device: Option<Vec<ProcedureFocalDevice>>,
    pub used: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
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
#[fhir_choice_element(base_name = "occurred")]
pub enum ProvenanceOccurred {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurredPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurredDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "occurred")]
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
    pub recorded: Option<Instant>,
    pub policy: Option<Vec<Uri>>,
    pub location: Option<Reference>,
    pub authorization: Option<Vec<CodeableReference>>,
    pub why: Option<Markdown>,
    pub activity: Option<CodeableConcept>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub patient: Option<Reference>,
    pub encounter: Option<Reference>,
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
    #[fhir_serde(rename = "disabledDisplay")]
    pub disabled_display: Option<Code>,
    pub required: Option<Boolean>,
    pub repeats: Option<Boolean>,
    #[fhir_serde(rename = "readOnly")]
    pub read_only: Option<Boolean>,
    #[fhir_serde(rename = "maxLength")]
    pub max_length: Option<Integer>,
    #[fhir_serde(rename = "answerConstraint")]
    pub answer_constraint: Option<Code>,
    #[fhir_serde(rename = "answerValueSet")]
    pub answer_value_set: Option<Canonical>,
    #[fhir_serde(rename = "answerOption")]
    pub answer_option: Option<Vec<QuestionnaireItemAnswerOption>>,
    pub initial: Option<Vec<QuestionnaireItemInitial>>,
    pub item: Option<Vec<QuestionnaireItem>>,
}

/// Choice of types for the value\[x\] field in QuestionnaireItemAnswerOption
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum QuestionnaireItemAnswerOptionValue {
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
#[fhir_resource(choice_elements = "value")]
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

/// Choice of types for the value\[x\] field in QuestionnaireItemInitial
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
pub struct QuestionnaireItemInitial {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub value: Option<QuestionnaireItemInitialValue>,
}

/// Choice of types for the versionAlgorithm\[x\] field in Questionnaire
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum QuestionnaireVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<QuestionnaireVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
#[fhir_choice_element(base_name = "answer")]
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
#[fhir_resource(choice_elements = "answer")]
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
    pub identifier: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub questionnaire: Canonical,
    pub status: Code,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub authored: Option<DateTime>,
    pub author: Option<Reference>,
    pub source: Option<Reference>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

/// Choice of types for the value\[x\] field in QuestionnaireResponseItemAnswer
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
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
pub struct QuestionnaireResponseItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: String,
    pub definition: Option<Vec<Uri>>,
    pub text: Option<String>,
    pub answer: Option<Vec<QuestionnaireResponseItemAnswer>>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

/// Choice of types for the date\[x\] field in RegulatedAuthorizationCase
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "date")]
pub enum RegulatedAuthorizationCaseDate {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "datePeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "dateDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "date")]
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
    pub indication: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "intendedUse")]
    pub intended_use: Option<CodeableConcept>,
    pub basis: Option<Vec<CodeableConcept>>,
    pub holder: Option<Reference>,
    pub regulator: Option<Reference>,
    #[fhir_serde(rename = "attachedDocument")]
    pub attached_document: Option<Vec<Reference>>,
    pub case: Option<RegulatedAuthorizationCase>,
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
    pub role: Option<Vec<CodeableConcept>>,
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
pub struct RequestOrchestrationActionCondition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    pub expression: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequestOrchestrationActionInput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub requirement: Option<DataRequirement>,
    #[fhir_serde(rename = "relatedData")]
    pub related_data: Option<Id>,
}

/// Choice of types for the timing\[x\] field in RequestOrchestrationAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "timing")]
pub enum RequestOrchestrationActionTiming {
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

/// Choice of types for the definition\[x\] field in RequestOrchestrationAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "definition")]
pub enum RequestOrchestrationActionDefinition {
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "definitionUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "timing,definition")]
pub struct RequestOrchestrationAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "linkId")]
    pub link_id: Option<String>,
    pub prefix: Option<String>,
    pub title: Option<String>,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "textEquivalent")]
    pub text_equivalent: Option<Markdown>,
    pub priority: Option<Code>,
    pub code: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    pub goal: Option<Vec<Reference>>,
    pub condition: Option<Vec<RequestOrchestrationActionCondition>>,
    pub input: Option<Vec<RequestOrchestrationActionInput>>,
    pub output: Option<Vec<RequestOrchestrationActionOutput>>,
    #[fhir_serde(rename = "relatedAction")]
    pub related_action: Option<Vec<RequestOrchestrationActionRelatedAction>>,
    #[fhir_serde(flatten)]
    pub timing: Option<RequestOrchestrationActionTiming>,
    pub location: Option<CodeableReference>,
    pub participant: Option<Vec<RequestOrchestrationActionParticipant>>,
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
    #[fhir_serde(flatten)]
    pub definition: Option<RequestOrchestrationActionDefinition>,
    pub transform: Option<Canonical>,
    #[fhir_serde(rename = "dynamicValue")]
    pub dynamic_value: Option<Vec<RequestOrchestrationActionDynamicValue>>,
    pub action: Option<Vec<RequestOrchestrationAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequestOrchestrationActionOutput {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub requirement: Option<DataRequirement>,
    #[fhir_serde(rename = "relatedData")]
    pub related_data: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequestOrchestrationActionDynamicValue {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub expression: Option<Expression>,
}

/// Choice of types for the actor\[x\] field in RequestOrchestrationActionParticipant
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "actor")]
pub enum RequestOrchestrationActionParticipantActor {
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "actorCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "actorReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "actor")]
pub struct RequestOrchestrationActionParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Code>,
    #[fhir_serde(rename = "typeCanonical")]
    pub type_canonical: Option<Canonical>,
    #[fhir_serde(rename = "typeReference")]
    pub type_reference: Option<Reference>,
    pub role: Option<CodeableConcept>,
    pub function: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub actor: Option<RequestOrchestrationActionParticipantActor>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequestOrchestration {
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
    pub reason: Option<Vec<CodeableReference>>,
    pub goal: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub action: Option<Vec<RequestOrchestrationAction>>,
}

/// Choice of types for the offset\[x\] field in RequestOrchestrationActionRelatedAction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "offset")]
pub enum RequestOrchestrationActionRelatedActionOffset {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "offset")]
pub struct RequestOrchestrationActionRelatedAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "targetId")]
    pub target_id: Id,
    pub relationship: Code,
    #[fhir_serde(rename = "endRelationship")]
    pub end_relationship: Option<Code>,
    #[fhir_serde(flatten)]
    pub offset: Option<RequestOrchestrationActionRelatedActionOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequirementsImports {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Canonical,
    pub key: Option<Vec<Id>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequirementsStatement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub key: Id,
    pub label: Option<String>,
    pub conformance: Option<Vec<Code>>,
    pub conditionality: Option<Boolean>,
    pub requirement: Markdown,
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<RequirementsStatementDerivedFrom>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<RequirementsStatementPartOf>,
    #[fhir_serde(rename = "satisfiedBy")]
    pub satisfied_by: Option<Vec<Url>>,
    pub reference: Option<Vec<Url>>,
    pub source: Option<Vec<Reference>>,
    pub actor: Option<Vec<Id>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in Requirements
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum RequirementsVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
pub struct Requirements {
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
    pub version_algorithm: Option<RequirementsVersionAlgorithm>,
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
    #[fhir_serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Canonical>>,
    pub imports: Option<Vec<RequirementsImports>>,
    pub reference: Option<Vec<Url>>,
    pub actor: Option<Vec<RequirementsActor>>,
    pub statement: Option<Vec<RequirementsStatement>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequirementsStatementDerivedFrom {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Canonical>,
    pub key: Id,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequirementsActor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Canonical,
    pub key: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct RequirementsStatementPartOf {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Canonical>,
    pub key: Id,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyRecruitment {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "targetNumber")]
    pub target_number: Option<UnsignedInt>,
    #[fhir_serde(rename = "actualNumber")]
    pub actual_number: Option<UnsignedInt>,
    pub eligibility: Option<Reference>,
    #[fhir_serde(rename = "actualGroup")]
    pub actual_group: Option<Reference>,
    pub description: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyObjectiveOutcomeMeasure {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub endpoint: Reference,
    pub population: Option<Reference>,
    pub intervention: Option<Reference>,
    pub comparator: Option<Reference>,
    #[fhir_serde(rename = "summaryMeasure")]
    pub summary_measure: Option<CodeableConcept>,
    #[fhir_serde(rename = "endpointAnalysisPlan")]
    pub endpoint_analysis_plan: Option<Reference>,
    #[fhir_serde(rename = "eventHandling")]
    pub event_handling: Option<Vec<ResearchStudyObjectiveOutcomeMeasureEventHandling>>,
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
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub label: Option<Vec<ResearchStudyLabel>>,
    pub protocol: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    #[fhir_serde(rename = "citeAs")]
    pub cite_as: Option<Markdown>,
    #[fhir_serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<ResearchStudyRelatesTo>>,
    pub date: Option<DateTime>,
    pub status: Code,
    #[fhir_serde(rename = "primaryPurposeType")]
    pub primary_purpose_type: Option<CodeableConcept>,
    pub phase: Option<CodeableConcept>,
    #[fhir_serde(rename = "studyDesign")]
    pub study_design: Option<Vec<CodeableConcept>>,
    pub focus: Option<Vec<CodeableReference>>,
    pub condition: Option<Vec<CodeableConcept>>,
    pub keyword: Option<Vec<CodeableConcept>>,
    pub region: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "descriptionSummary")]
    pub description_summary: Option<Markdown>,
    pub description: Option<Markdown>,
    pub period: Option<Period>,
    pub site: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub classifier: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "associatedParty")]
    pub associated_party: Option<Vec<ResearchStudyAssociatedParty>>,
    #[fhir_serde(rename = "progressStatus")]
    pub progress_status: Option<Vec<ResearchStudyProgressStatus>>,
    #[fhir_serde(rename = "whyStopped")]
    pub why_stopped: Option<CodeableConcept>,
    pub recruitment: Option<ResearchStudyRecruitment>,
    #[fhir_serde(rename = "comparisonGroup")]
    pub comparison_group: Option<Vec<ResearchStudyComparisonGroup>>,
    pub objective: Option<Vec<ResearchStudyObjective>>,
    pub result: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyLabel {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub value: Option<String>,
    pub language: Option<Code>,
}

/// Choice of types for the target\[x\] field in ResearchStudyRelatesTo
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "target")]
pub enum ResearchStudyRelatesToTarget {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[fhir_serde(rename = "targetAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "targetCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "targetReference")]
    Reference(Reference),
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "targetMarkdown")]
    Markdown(Markdown),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "target")]
pub struct ResearchStudyRelatesTo {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    #[fhir_serde(flatten)]
    pub target: Option<ResearchStudyRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyProgressStatus {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub state: CodeableConcept,
    pub actual: Option<Boolean>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyComparisonGroup {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "targetNumber")]
    pub target_number: Option<UnsignedInt>,
    #[fhir_serde(rename = "actualNumber")]
    pub actual_number: Option<UnsignedInt>,
    pub eligibility: Option<Reference>,
    #[fhir_serde(rename = "observedGroup")]
    pub observed_group: Option<Reference>,
    pub description: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyAssociatedParty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub role: CodeableConcept,
    pub period: Option<Vec<Period>>,
    pub classifier: Option<Vec<CodeableConcept>>,
    pub party: Option<Reference>,
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
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "outcomeMeasure")]
    pub outcome_measure: Option<Vec<ResearchStudyObjectiveOutcomeMeasure>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchStudyObjectiveOutcomeMeasureEventHandling {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub event: Option<CodeableConcept>,
    pub group: Option<CodeableConcept>,
    pub handling: Option<CodeableConcept>,
    pub description: Option<Markdown>,
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
    pub subject: Reference,
    #[fhir_serde(rename = "subjectState")]
    pub subject_state: Option<Vec<ResearchSubjectSubjectState>>,
    #[fhir_serde(rename = "subjectMilestone")]
    pub subject_milestone: Option<Vec<ResearchSubjectSubjectMilestone>>,
    #[fhir_serde(rename = "assignedComparisonGroup")]
    pub assigned_comparison_group: Option<Id>,
    #[fhir_serde(rename = "actualComparisonGroup")]
    pub actual_comparison_group: Option<Id>,
    pub consent: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchSubjectSubjectState {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(rename = "startDate")]
    pub start_date: DateTime,
    #[fhir_serde(rename = "endDate")]
    pub end_date: Option<DateTime>,
    pub reason: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ResearchSubjectSubjectMilestone {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub milestone: CodeableConcept,
    pub date: Option<DateTime>,
    pub reason: Option<Vec<CodeableConcept>>,
}

/// Choice of types for the occurrence\[x\] field in RiskAssessment
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
pub enum RiskAssessmentOccurrence {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "occurrence")]
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
    pub reason: Option<Vec<CodeableReference>>,
    pub basis: Option<Vec<Reference>>,
    pub prediction: Option<Vec<RiskAssessmentPrediction>>,
    pub mitigation: Option<String>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the probability\[x\] field in RiskAssessmentPrediction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "probability")]
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
#[fhir_choice_element(base_name = "when")]
pub enum RiskAssessmentPredictionWhen {
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "whenPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "whenRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "probability,when")]
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
    pub service_type: Option<Vec<CodeableReference>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub actor: Option<Vec<Reference>>,
    #[fhir_serde(rename = "planningHorizon")]
    pub planning_horizon: Option<Period>,
    pub comment: Option<Markdown>,
}

/// Choice of types for the versionAlgorithm\[x\] field in SearchParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum SearchParameterVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<SearchParameterVersionAlgorithm>,
    pub name: String,
    pub title: Option<String>,
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
    pub copyright: Option<Markdown>,
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    pub code: Code,
    #[fhir_serde(rename = "aliasCode")]
    pub alias_code: Option<Vec<Code>>,
    pub base: Option<Vec<Code>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub expression: Option<String>,
    #[fhir_serde(rename = "processingMode")]
    pub processing_mode: Option<Code>,
    pub constraint: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SearchParameterComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub definition: Canonical,
    pub expression: String,
}

/// Choice of types for the quantity\[x\] field in ServiceRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "quantity")]
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
#[fhir_choice_element(base_name = "occurrence")]
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "quantity,occurrence")]
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
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    pub code: Option<CodeableReference>,
    #[fhir_serde(rename = "orderDetail")]
    pub order_detail: Option<Vec<ServiceRequestOrderDetail>>,
    #[fhir_serde(flatten)]
    pub quantity: Option<ServiceRequestQuantity>,
    pub subject: Reference,
    pub focus: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<ServiceRequestOccurrence>,
    #[fhir_serde(rename = "asNeeded")]
    pub as_needed: Option<Boolean>,
    #[fhir_serde(rename = "asNeededFor")]
    pub as_needed_for: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    #[fhir_serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub performer: Option<Vec<Reference>>,
    pub location: Option<Vec<CodeableReference>>,
    pub reason: Option<Vec<CodeableReference>>,
    pub insurance: Option<Vec<Reference>>,
    #[fhir_serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<CodeableReference>>,
    pub specimen: Option<Vec<Reference>>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "bodyStructure")]
    pub body_structure: Option<Reference>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "patientInstruction")]
    pub patient_instruction: Option<Vec<ServiceRequestPatientInstruction>>,
    #[fhir_serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ServiceRequestOrderDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "parameterFocus")]
    pub parameter_focus: Option<CodeableReference>,
    pub parameter: Option<Vec<ServiceRequestOrderDetailParameter>>,
}

/// Choice of types for the instruction\[x\] field in ServiceRequestPatientInstruction
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "instruction")]
pub enum ServiceRequestPatientInstructionInstruction {
    /// Variant accepting the Markdown type.
    #[fhir_serde(rename = "instructionMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "instructionReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "instruction")]
pub struct ServiceRequestPatientInstruction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub instruction: Option<ServiceRequestPatientInstructionInstruction>,
}

/// Choice of types for the value\[x\] field in ServiceRequestOrderDetailParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum ServiceRequestOrderDetailParameterValue {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[fhir_serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Boolean type.
    #[fhir_serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "valuePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct ServiceRequestOrderDetailParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[fhir_serde(flatten)]
    pub value: Option<ServiceRequestOrderDetailParameterValue>,
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
    pub service_type: Option<Vec<CodeableReference>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "appointmentType")]
    pub appointment_type: Option<Vec<CodeableConcept>>,
    pub schedule: Reference,
    pub status: Code,
    pub start: Instant,
    pub end: Instant,
    pub overbooked: Option<Boolean>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenFeature {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SpecimenContainer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub device: CodeableReference,
    #[fhir_serde(rename = "specimenQuantity")]
    pub specimen_quantity: Option<Quantity>,
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
    pub combined: Option<Code>,
    pub role: Option<Vec<CodeableConcept>>,
    pub feature: Option<Vec<SpecimenFeature>>,
    pub collection: Option<SpecimenCollection>,
    pub processing: Option<Vec<SpecimenProcessing>>,
    pub container: Option<Vec<SpecimenContainer>>,
    pub condition: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the collected\[x\] field in SpecimenCollection
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "collected")]
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
#[fhir_choice_element(base_name = "fastingStatus")]
pub enum SpecimenCollectionFastingStatus {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "fastingStatusCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "fastingStatusDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "collected,fastingStatus")]
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
    pub device: Option<CodeableReference>,
    pub procedure: Option<Reference>,
    #[fhir_serde(rename = "bodySite")]
    pub body_site: Option<CodeableReference>,
    #[fhir_serde(flatten)]
    pub fasting_status: Option<SpecimenCollectionFastingStatus>,
}

/// Choice of types for the time\[x\] field in SpecimenProcessing
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "time")]
pub enum SpecimenProcessingTime {
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[fhir_serde(rename = "timePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "timeDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "time")]
pub struct SpecimenProcessing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub method: Option<CodeableConcept>,
    pub performer: Option<Reference>,
    pub device: Option<CodeableReference>,
    pub additive: Option<Vec<Reference>>,
    #[fhir_serde(flatten)]
    pub time: Option<SpecimenProcessingTime>,
}

/// Choice of types for the minimumVolume\[x\] field in SpecimenDefinitionTypeTestedContainer
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "minimumVolume")]
pub enum SpecimenDefinitionTypeTestedContainerMinimumVolume {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "minimumVolumeQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "minimumVolumeString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "minimumVolume")]
pub struct SpecimenDefinitionTypeTestedContainer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub material: Option<CodeableConcept>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub cap: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub capacity: Option<Quantity>,
    #[fhir_serde(flatten)]
    pub minimum_volume: Option<SpecimenDefinitionTypeTestedContainerMinimumVolume>,
    pub additive: Option<Vec<SpecimenDefinitionTypeTestedContainerAdditive>>,
    pub preparation: Option<Markdown>,
}

/// Choice of types for the additive\[x\] field in SpecimenDefinitionTypeTestedContainerAdditive
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "additive")]
pub enum SpecimenDefinitionTypeTestedContainerAdditiveAdditive {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "additive")]
pub struct SpecimenDefinitionTypeTestedContainerAdditive {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub additive: Option<SpecimenDefinitionTypeTestedContainerAdditiveAdditive>,
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
    pub instruction: Option<Markdown>,
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
    pub requirement: Option<Markdown>,
    #[fhir_serde(rename = "retentionTime")]
    pub retention_time: Option<Duration>,
    #[fhir_serde(rename = "singleUse")]
    pub single_use: Option<Boolean>,
    #[fhir_serde(rename = "rejectionCriterion")]
    pub rejection_criterion: Option<Vec<CodeableConcept>>,
    pub handling: Option<Vec<SpecimenDefinitionTypeTestedHandling>>,
    #[fhir_serde(rename = "testingDestination")]
    pub testing_destination: Option<Vec<CodeableConcept>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in SpecimenDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum SpecimenDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

/// Choice of types for the subject\[x\] field in SpecimenDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
pub enum SpecimenDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm,subject")]
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
    pub url: Option<Uri>,
    pub identifier: Option<Identifier>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<SpecimenDefinitionVersionAlgorithm>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[fhir_serde(rename = "derivedFromCanonical")]
    pub derived_from_canonical: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "derivedFromUri")]
    pub derived_from_uri: Option<Vec<Uri>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[fhir_serde(flatten)]
    pub subject: Option<SpecimenDefinitionSubject>,
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
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
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
pub struct StructureDefinitionDifferential {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in StructureDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum StructureDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<StructureDefinitionVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
pub struct StructureDefinitionSnapshot {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in StructureMap
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum StructureMapVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<StructureMapVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    pub structure: Option<Vec<StructureMapStructure>>,
    pub import: Option<Vec<Canonical>>,
    #[fhir_serde(rename = "const")]
    pub r#const: Option<Vec<StructureMapConst>>,
    pub group: Option<Vec<StructureMapGroup>>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRule {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<Id>,
    pub source: Option<Vec<StructureMapGroupRuleSource>>,
    pub target: Option<Vec<StructureMapGroupRuleTarget>>,
    pub rule: Option<Vec<StructureMapGroupRule>>,
    pub dependent: Option<Vec<StructureMapGroupRuleDependent>>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRuleDependent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub parameter: Option<Vec<StructureMapGroupRuleTargetParameter>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRuleSource {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Id,
    pub min: Option<UnsignedInt>,
    pub max: Option<String>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<String>,
    #[fhir_serde(rename = "defaultValue")]
    pub default_value: Option<String>,
    pub element: Option<String>,
    #[fhir_serde(rename = "listMode")]
    pub list_mode: Option<Code>,
    pub variable: Option<Id>,
    pub condition: Option<String>,
    pub check: Option<String>,
    #[fhir_serde(rename = "logMessage")]
    pub log_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapGroupRuleTarget {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Option<String>,
    pub element: Option<String>,
    pub variable: Option<Id>,
    #[fhir_serde(rename = "listMode")]
    pub list_mode: Option<Vec<Code>>,
    #[fhir_serde(rename = "listRuleId")]
    pub list_rule_id: Option<Id>,
    pub transform: Option<Code>,
    pub parameter: Option<Vec<StructureMapGroupRuleTargetParameter>>,
}

/// Choice of types for the value\[x\] field in StructureMapGroupRuleTargetParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Date type.
    #[fhir_serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Time type.
    #[fhir_serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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
    pub type_mode: Option<Code>,
    pub documentation: Option<String>,
    pub input: Option<Vec<StructureMapGroupInput>>,
    pub rule: Option<Vec<StructureMapGroupRule>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct StructureMapConst {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<Id>,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionFilterBy {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub resource: Option<Uri>,
    #[fhir_serde(rename = "filterParameter")]
    pub filter_parameter: String,
    pub comparator: Option<Code>,
    pub modifier: Option<Code>,
    pub value: String,
    pub event: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub value: String,
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
    pub identifier: Option<Vec<Identifier>>,
    pub name: Option<String>,
    pub status: Code,
    pub topic: Canonical,
    pub contact: Option<Vec<ContactPoint>>,
    pub end: Option<Instant>,
    #[fhir_serde(rename = "managingEntity")]
    pub managing_entity: Option<Reference>,
    pub reason: Option<String>,
    #[fhir_serde(rename = "filterBy")]
    pub filter_by: Option<Vec<SubscriptionFilterBy>>,
    #[fhir_serde(rename = "channelType")]
    pub channel_type: Coding,
    pub endpoint: Option<Url>,
    pub parameter: Option<Vec<SubscriptionParameter>>,
    #[fhir_serde(rename = "heartbeatPeriod")]
    pub heartbeat_period: Option<UnsignedInt>,
    pub timeout: Option<UnsignedInt>,
    #[fhir_serde(rename = "contentType")]
    pub content_type: Option<Code>,
    pub content: Option<Code>,
    #[fhir_serde(rename = "maxCount")]
    pub max_count: Option<PositiveInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionStatusNotificationEvent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "eventNumber")]
    pub event_number: Integer64,
    #[fhir_serde(rename = "triggerEvent")]
    pub trigger_event: Option<Vec<CodeableConcept>>,
    pub timestamp: Option<Instant>,
    pub focus: Option<Reference>,
    #[fhir_serde(rename = "additionalContext")]
    pub additional_context: Option<Vec<Reference>>,
    #[fhir_serde(rename = "relatedQuery")]
    pub related_query: Option<Vec<SubscriptionStatusNotificationEventRelatedQuery>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionStatusNotificationEventRelatedQuery {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "queryType")]
    pub query_type: Option<Coding>,
    pub query: String,
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
    pub events_since_subscription_start: Option<Integer64>,
    #[fhir_serde(rename = "notificationEvent")]
    pub notification_event: Option<Vec<SubscriptionStatusNotificationEvent>>,
    pub subscription: Reference,
    pub topic: Option<Canonical>,
    pub error: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicTriggerNotificationShape {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub resource: Uri,
    pub include: Option<Vec<String>>,
    #[fhir_serde(rename = "revInclude")]
    pub rev_include: Option<Vec<String>>,
    #[fhir_serde(rename = "relatedQuery")]
    pub related_query: Option<Vec<SubscriptionTopicTriggerNotificationShapeRelatedQuery>>,
}

/// Choice of types for the versionAlgorithm\[x\] field in SubscriptionTopic
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum SubscriptionTopicVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<SubscriptionTopicVersionAlgorithm>,
    pub name: Option<String>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    #[fhir_serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[fhir_serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[fhir_serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub trigger: Option<Vec<SubscriptionTopicTrigger>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicTriggerCanFilterBy {
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
    pub comparator: Option<Vec<Code>>,
    pub modifier: Option<Vec<Code>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicTrigger {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub resource: Uri,
    #[fhir_serde(rename = "supportedInteraction")]
    pub supported_interaction: Option<Vec<Code>>,
    #[fhir_serde(rename = "queryCriteria")]
    pub query_criteria: Option<SubscriptionTopicTriggerQueryCriteria>,
    #[fhir_serde(rename = "fhirPathCriteria")]
    pub fhir_path_criteria: Option<String>,
    pub event: Option<CodeableConcept>,
    #[fhir_serde(rename = "canFilterBy")]
    pub can_filter_by: Option<Vec<SubscriptionTopicTriggerCanFilterBy>>,
    #[fhir_serde(rename = "notificationShape")]
    pub notification_shape: Option<Vec<SubscriptionTopicTriggerNotificationShape>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubscriptionTopicTriggerQueryCriteria {
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
pub struct SubscriptionTopicTriggerNotificationShapeRelatedQuery {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "queryType")]
    pub query_type: Option<Coding>,
    pub query: String,
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
    pub code: CodeableReference,
    pub description: Option<Markdown>,
    pub expiry: Option<DateTime>,
    pub quantity: Option<Quantity>,
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
    pub characterization: Option<Vec<SubstanceDefinitionCharacterization>>,
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
pub struct SubstanceDefinitionCharacterization {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub technique: Option<CodeableConcept>,
    pub form: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub file: Option<Vec<Attachment>>,
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

/// Choice of types for the amount\[x\] field in SubstanceDefinitionMoiety
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "amount")]
pub enum SubstanceDefinitionMoietyAmount {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "amount")]
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

/// Choice of types for the value\[x\] field in SubstanceDefinitionProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
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

/// Choice of types for the substanceDefinition\[x\] field in SubstanceDefinitionRelationship
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "substanceDefinition")]
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
#[fhir_choice_element(base_name = "amount")]
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
#[fhir_resource(choice_elements = "substanceDefinition,amount")]
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceNucleicAcidSubunit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub subunit: Option<Integer>,
    pub sequence: Option<String>,
    pub length: Option<Integer>,
    #[fhir_serde(rename = "sequenceAttachment")]
    pub sequence_attachment: Option<Attachment>,
    #[fhir_serde(rename = "fivePrime")]
    pub five_prime: Option<CodeableConcept>,
    #[fhir_serde(rename = "threePrime")]
    pub three_prime: Option<CodeableConcept>,
    pub linkage: Option<Vec<SubstanceNucleicAcidSubunitLinkage>>,
    pub sugar: Option<Vec<SubstanceNucleicAcidSubunitSugar>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceNucleicAcid {
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
    #[fhir_serde(rename = "sequenceType")]
    pub sequence_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "numberOfSubunits")]
    pub number_of_subunits: Option<Integer>,
    #[fhir_serde(rename = "areaOfHybridisation")]
    pub area_of_hybridisation: Option<String>,
    #[fhir_serde(rename = "oligoNucleotideType")]
    pub oligo_nucleotide_type: Option<CodeableConcept>,
    pub subunit: Option<Vec<SubstanceNucleicAcidSubunit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceNucleicAcidSubunitLinkage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub connectivity: Option<String>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    #[fhir_serde(rename = "residueSite")]
    pub residue_site: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceNucleicAcidSubunitSugar {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    #[fhir_serde(rename = "residueSite")]
    pub residue_site: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstancePolymerRepeatRepeatUnit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub unit: Option<String>,
    pub orientation: Option<CodeableConcept>,
    pub amount: Option<Integer>,
    #[fhir_serde(rename = "degreeOfPolymerisation")]
    pub degree_of_polymerisation:
        Option<Vec<SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation>>,
    #[fhir_serde(rename = "structuralRepresentation")]
    pub structural_representation:
        Option<Vec<SubstancePolymerRepeatRepeatUnitStructuralRepresentation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstancePolymerMonomerSet {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "ratioType")]
    pub ratio_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "startingMaterial")]
    pub starting_material: Option<Vec<SubstancePolymerMonomerSetStartingMaterial>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstancePolymerMonomerSetStartingMaterial {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[fhir_serde(rename = "isDefining")]
    pub is_defining: Option<Boolean>,
    pub amount: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstancePolymerRepeat {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "averageMolecularFormula")]
    pub average_molecular_formula: Option<String>,
    #[fhir_serde(rename = "repeatUnitAmountType")]
    pub repeat_unit_amount_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "repeatUnit")]
    pub repeat_unit: Option<Vec<SubstancePolymerRepeatRepeatUnit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstancePolymer {
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
    pub class: Option<CodeableConcept>,
    pub geometry: Option<CodeableConcept>,
    #[fhir_serde(rename = "copolymerConnectivity")]
    pub copolymer_connectivity: Option<Vec<CodeableConcept>>,
    pub modification: Option<String>,
    #[fhir_serde(rename = "monomerSet")]
    pub monomer_set: Option<Vec<SubstancePolymerMonomerSet>>,
    pub repeat: Option<Vec<SubstancePolymerRepeat>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub average: Option<Integer>,
    pub low: Option<Integer>,
    pub high: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstancePolymerRepeatRepeatUnitStructuralRepresentation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub representation: Option<String>,
    pub format: Option<CodeableConcept>,
    pub attachment: Option<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceProtein {
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
    #[fhir_serde(rename = "sequenceType")]
    pub sequence_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "numberOfSubunits")]
    pub number_of_subunits: Option<Integer>,
    #[fhir_serde(rename = "disulfideLinkage")]
    pub disulfide_linkage: Option<Vec<String>>,
    pub subunit: Option<Vec<SubstanceProteinSubunit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceProteinSubunit {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub subunit: Option<Integer>,
    pub sequence: Option<String>,
    pub length: Option<Integer>,
    #[fhir_serde(rename = "sequenceAttachment")]
    pub sequence_attachment: Option<Attachment>,
    #[fhir_serde(rename = "nTerminalModificationId")]
    pub n_terminal_modification_id: Option<Identifier>,
    #[fhir_serde(rename = "nTerminalModification")]
    pub n_terminal_modification: Option<String>,
    #[fhir_serde(rename = "cTerminalModificationId")]
    pub c_terminal_modification_id: Option<Identifier>,
    #[fhir_serde(rename = "cTerminalModification")]
    pub c_terminal_modification: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceReferenceInformationGene {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "geneSequenceOrigin")]
    pub gene_sequence_origin: Option<CodeableConcept>,
    pub gene: Option<CodeableConcept>,
    pub source: Option<Vec<Reference>>,
}

/// Choice of types for the amount\[x\] field in SubstanceReferenceInformationTarget
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "amount")]
pub enum SubstanceReferenceInformationTargetAmount {
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "amountRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "amount")]
pub struct SubstanceReferenceInformationTarget {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Option<Identifier>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub interaction: Option<CodeableConcept>,
    pub organism: Option<CodeableConcept>,
    #[fhir_serde(rename = "organismType")]
    pub organism_type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub amount: Option<SubstanceReferenceInformationTargetAmount>,
    #[fhir_serde(rename = "amountType")]
    pub amount_type: Option<CodeableConcept>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceReferenceInformation {
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
    pub comment: Option<String>,
    pub gene: Option<Vec<SubstanceReferenceInformationGene>>,
    #[fhir_serde(rename = "geneElement")]
    pub gene_element: Option<Vec<SubstanceReferenceInformationGeneElement>>,
    pub target: Option<Vec<SubstanceReferenceInformationTarget>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceReferenceInformationGeneElement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub element: Option<Identifier>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceSourceMaterialOrganism {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub family: Option<CodeableConcept>,
    pub genus: Option<CodeableConcept>,
    pub species: Option<CodeableConcept>,
    #[fhir_serde(rename = "intraspecificType")]
    pub intraspecific_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "intraspecificDescription")]
    pub intraspecific_description: Option<String>,
    pub author: Option<Vec<SubstanceSourceMaterialOrganismAuthor>>,
    pub hybrid: Option<SubstanceSourceMaterialOrganismHybrid>,
    #[fhir_serde(rename = "organismGeneral")]
    pub organism_general: Option<SubstanceSourceMaterialOrganismOrganismGeneral>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceSourceMaterialFractionDescription {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub fraction: Option<String>,
    #[fhir_serde(rename = "materialType")]
    pub material_type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceSourceMaterialOrganismHybrid {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "maternalOrganismId")]
    pub maternal_organism_id: Option<String>,
    #[fhir_serde(rename = "maternalOrganismName")]
    pub maternal_organism_name: Option<String>,
    #[fhir_serde(rename = "paternalOrganismId")]
    pub paternal_organism_id: Option<String>,
    #[fhir_serde(rename = "paternalOrganismName")]
    pub paternal_organism_name: Option<String>,
    #[fhir_serde(rename = "hybridType")]
    pub hybrid_type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceSourceMaterialOrganismOrganismGeneral {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kingdom: Option<CodeableConcept>,
    pub phylum: Option<CodeableConcept>,
    pub class: Option<CodeableConcept>,
    pub order: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceSourceMaterialPartDescription {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub part: Option<CodeableConcept>,
    #[fhir_serde(rename = "partLocation")]
    pub part_location: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceSourceMaterialOrganismAuthor {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "authorType")]
    pub author_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "authorDescription")]
    pub author_description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SubstanceSourceMaterial {
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
    #[fhir_serde(rename = "sourceMaterialClass")]
    pub source_material_class: Option<CodeableConcept>,
    #[fhir_serde(rename = "sourceMaterialType")]
    pub source_material_type: Option<CodeableConcept>,
    #[fhir_serde(rename = "sourceMaterialState")]
    pub source_material_state: Option<CodeableConcept>,
    #[fhir_serde(rename = "organismId")]
    pub organism_id: Option<Identifier>,
    #[fhir_serde(rename = "organismName")]
    pub organism_name: Option<String>,
    #[fhir_serde(rename = "parentSubstanceId")]
    pub parent_substance_id: Option<Vec<Identifier>>,
    #[fhir_serde(rename = "parentSubstanceName")]
    pub parent_substance_name: Option<Vec<String>>,
    #[fhir_serde(rename = "countryOfOrigin")]
    pub country_of_origin: Option<Vec<CodeableConcept>>,
    #[fhir_serde(rename = "geographicalLocation")]
    pub geographical_location: Option<Vec<String>>,
    #[fhir_serde(rename = "developmentStage")]
    pub development_stage: Option<CodeableConcept>,
    #[fhir_serde(rename = "fractionDescription")]
    pub fraction_description: Option<Vec<SubstanceSourceMaterialFractionDescription>>,
    pub organism: Option<SubstanceSourceMaterialOrganism>,
    #[fhir_serde(rename = "partDescription")]
    pub part_description: Option<Vec<SubstanceSourceMaterialPartDescription>>,
}

/// Choice of types for the occurrence\[x\] field in SupplyDelivery
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
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
#[fhir_resource(choice_elements = "occurrence")]
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
    pub status: Code,
    pub subject: Option<Reference>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub stage: CodeableConcept,
    #[fhir_serde(rename = "suppliedItem")]
    pub supplied_item: Option<Vec<SupplyDeliverySuppliedItem>>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<SupplyDeliveryOccurrence>,
    pub supplier: Option<Reference>,
    pub destination: Option<Reference>,
    pub receiver: Option<Vec<Reference>>,
}

/// Choice of types for the item\[x\] field in SupplyDeliverySuppliedItem
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "item")]
pub enum SupplyDeliverySuppliedItemItem {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "item")]
pub struct SupplyDeliverySuppliedItem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    pub condition: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub item: Option<SupplyDeliverySuppliedItemItem>,
}

/// Choice of types for the occurrence\[x\] field in SupplyRequest
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "occurrence")]
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
#[fhir_resource(choice_elements = "occurrence")]
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
    pub intent: Code,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub category: Option<CodeableConcept>,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "deliverFor")]
    pub deliver_for: Option<Reference>,
    pub item: CodeableReference,
    pub quantity: Quantity,
    pub parameter: Option<Vec<SupplyRequestParameter>>,
    #[fhir_serde(flatten)]
    pub occurrence: Option<SupplyRequestOccurrence>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub supplier: Option<Vec<Reference>>,
    pub reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "deliverFrom")]
    pub deliver_from: Option<Reference>,
    #[fhir_serde(rename = "deliverTo")]
    pub deliver_to: Option<Reference>,
}

/// Choice of types for the value\[x\] field in SupplyRequestParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
pub struct SupplyRequestParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub value: Option<SupplyRequestParameterValue>,
}

/// Choice of types for the value\[x\] field in TaskInput
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "valueInteger64")]
    Integer64(Integer64),
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
    /// Variant accepting the Availability type.
    #[fhir_serde(rename = "valueAvailability")]
    Availability(Availability),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "valueExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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

/// Choice of types for the value\[x\] field in TaskOutput
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "valueInteger64")]
    Integer64(Integer64),
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
    /// Variant accepting the Availability type.
    #[fhir_serde(rename = "valueAvailability")]
    Availability(Availability),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "valueExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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
pub struct TaskPerformer {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
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
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableReference>>,
    #[fhir_serde(rename = "businessStatus")]
    pub business_status: Option<CodeableConcept>,
    pub intent: Code,
    pub priority: Option<Code>,
    #[fhir_serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub focus: Option<Vec<Reference>>,
    #[fhir_serde(rename = "for")]
    pub r#for: Option<Reference>,
    pub encounter: Option<Reference>,
    #[fhir_serde(rename = "requestedPeriod")]
    pub requested_period: Option<Period>,
    #[fhir_serde(rename = "executionPeriod")]
    pub execution_period: Option<Period>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    #[fhir_serde(rename = "lastModified")]
    pub last_modified: Option<DateTime>,
    pub requester: Option<Reference>,
    #[fhir_serde(rename = "requestedPerformer")]
    pub requested_performer: Option<Vec<CodeableReference>>,
    pub owner: Option<Reference>,
    pub performer: Option<Vec<TaskPerformer>>,
    pub location: Option<Reference>,
    pub reason: Option<Vec<CodeableReference>>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
    pub restriction: Option<TaskRestriction>,
    pub input: Option<Vec<TaskInput>>,
    pub output: Option<Vec<TaskOutput>>,
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
pub struct TerminologyCapabilitiesCodeSystemVersionFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub op: Option<Vec<Code>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesImplementation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Markdown,
    pub url: Option<Url>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TerminologyCapabilitiesSupplements {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub globals: Option<Code>,
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
pub struct TerminologyCapabilitiesValidateCode {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub translations: Boolean,
}

/// Choice of types for the versionAlgorithm\[x\] field in TerminologyCapabilities
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum TerminologyCapabilitiesVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<TerminologyCapabilitiesVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    pub kind: Code,
    pub software: Option<TerminologyCapabilitiesSoftware>,
    pub implementation: Option<TerminologyCapabilitiesImplementation>,
    #[fhir_serde(rename = "lockedDate")]
    pub locked_date: Option<Boolean>,
    #[fhir_serde(rename = "codeSystem")]
    pub code_system: Option<Vec<TerminologyCapabilitiesCodeSystem>>,
    pub supplements: Option<TerminologyCapabilitiesSupplements>,
    pub expansion: Option<TerminologyCapabilitiesExpansion>,
    #[fhir_serde(rename = "codeSearch")]
    pub code_search: Option<Code>,
    #[fhir_serde(rename = "validateCode")]
    pub validate_code: Option<TerminologyCapabilitiesValidateCode>,
    pub translation: Option<TerminologyCapabilitiesTranslation>,
    pub closure: Option<TerminologyCapabilitiesClosure>,
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
pub struct TerminologyCapabilitiesCodeSystem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Option<Canonical>,
    pub version: Option<Vec<TerminologyCapabilitiesCodeSystemVersion>>,
    pub content: Code,
    pub subsumption: Option<Boolean>,
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
pub struct TerminologyCapabilitiesExpansionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    pub documentation: Option<Markdown>,
}

/// Choice of types for the versionAlgorithm\[x\] field in TestPlan
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum TestPlanVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
pub struct TestPlan {
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
    pub version_algorithm: Option<TestPlanVersionAlgorithm>,
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
    pub scope: Option<Vec<TestPlanScope>>,
    pub dependency: Option<Vec<TestPlanDependency>>,
    pub runner: Url,
    pub mode: Option<Vec<TestPlanMode>>,
    pub parameter: Option<Vec<TestPlanParameter>>,
    pub suite: Option<Vec<TestPlanSuite>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestPlanDependency {
    pub reference: Canonical,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestPlanSuiteInput {
    pub name: Option<String>,
    pub file: Option<String>,
    pub resource: Option<Resource>,
    pub mode: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestPlanMode {
    pub code: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestPlanScope {
    pub reference: Option<Canonical>,
    pub description: Option<String>,
}

/// Choice of types for the value\[x\] field in TestPlanParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum TestPlanParameterValue {
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
    /// Variant accepting the DateTime type.
    #[fhir_serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[fhir_serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Expression type.
    #[fhir_serde(rename = "valueExpression")]
    Expression(Expression),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
pub struct TestPlanParameter {
    pub name: String,
    #[fhir_serde(flatten)]
    pub value: Option<TestPlanParameterValue>,
    pub mode: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestPlanSuiteTestAssertion {
    pub focus: Option<String>,
    pub severity: Option<Code>,
    pub expression: Option<Expression>,
    pub human: Option<String>,
    pub mode: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestPlanSuiteTest {
    pub name: String,
    pub description: Option<String>,
    pub operation: Option<Code>,
    pub mode: Option<Code>,
    pub parameter: Option<Vec<TestPlanParameter>>,
    pub input: Option<Vec<TestPlanSuiteInput>>,
    pub expected: Option<Vec<TestPlanSuiteInput>>,
    pub assertion: Option<Vec<TestPlanSuiteTestAssertion>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestPlanSuite {
    pub name: String,
    pub description: Option<String>,
    pub mode: Option<Code>,
    pub input: Option<Vec<TestPlanSuiteInput>>,
    pub parameter: Option<Vec<TestPlanParameter>>,
    pub test: Option<Vec<TestPlanSuiteTest>>,
    pub suite: Option<Vec<TestPlanSuite>>,
    pub plan: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportTeardown {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestReportTeardownAction>>,
}

/// Choice of types for the link\[x\] field in TestReportSetupActionAssertRequirement
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "link")]
pub enum TestReportSetupActionAssertRequirementLink {
    /// Variant accepting the Uri type.
    #[fhir_serde(rename = "linkUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[fhir_serde(rename = "linkCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "link")]
pub struct TestReportSetupActionAssertRequirement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(flatten)]
    pub link: Option<TestReportSetupActionAssertRequirementLink>,
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
    pub test_script: Canonical,
    pub result: Code,
    pub score: Option<Decimal>,
    pub tester: Option<String>,
    pub issued: Option<DateTime>,
    pub participant: Option<Vec<TestReportParticipant>>,
    pub parameter: Option<Vec<TestReportParameter>>,
    pub setup: Option<TestReportSetup>,
    pub test: Option<Vec<TestReportTest>>,
    pub teardown: Option<TestReportTeardown>,
    #[fhir_serde(rename = "presentedForm")]
    pub presented_form: Option<Attachment>,
    pub log: Option<Attachment>,
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
pub struct TestReportTestAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestReportSetupActionOperation>,
    pub assert: Option<TestReportSetupActionAssert>,
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
pub struct TestReportSetupActionAssert {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    pub message: Option<Markdown>,
    pub detail: Option<String>,
    pub requirement: Option<Vec<TestReportSetupActionAssertRequirement>>,
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
pub struct TestReportParticipant {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub uri: Option<Uri>,
    pub version: Option<Uri>,
    pub display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestReportTest {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub result: Option<Code>,
    pub period: Option<Period>,
    pub action: Option<Vec<TestReportTestAction>>,
    pub log: Option<Attachment>,
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
pub struct TestScriptFixture {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub autocreate: Boolean,
    pub autodelete: Boolean,
    pub resource: Option<Reference>,
    pub expression: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptTestAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub common: Option<TestScriptSetupActionCommon>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupActionAssertRequirement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Canonical,
    pub key: Id,
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
pub struct TestScriptTeardownAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub common: Option<TestScriptSetupActionCommon>,
    pub operation: TestScriptSetupActionOperation,
}

/// Choice of types for the versionAlgorithm\[x\] field in TestScript
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum TestScriptVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<TestScriptVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    #[fhir_serde(rename = "testSystem")]
    pub test_system: Option<Vec<TestScriptTestSystem>>,
    pub metadata: Option<TestScriptMetadata>,
    pub scope: Option<Vec<TestScriptScope>>,
    pub fixture: Option<Vec<TestScriptFixture>>,
    pub profile: Option<Vec<Canonical>>,
    pub variable: Option<Vec<TestScriptVariable>>,
    pub setup: Option<TestScriptSetup>,
    pub test: Option<Vec<TestScriptTest>>,
    pub teardown: Option<TestScriptTeardown>,
    pub common: Option<Vec<TestScriptCommon>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupActionOperation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Coding>,
    pub resource: Option<Uri>,
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
pub struct TestScriptSetupAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub common: Option<TestScriptSetupActionCommon>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupActionCommonParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupActionCommon {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "testScript")]
    pub test_script: Option<Canonical>,
    #[fhir_serde(rename = "keyRef")]
    pub key_ref: Id,
    pub parameter: Option<Vec<TestScriptSetupActionCommonParameter>>,
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
pub struct TestScriptVariable {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(rename = "defaultValue")]
    pub default_value: Option<String>,
    pub description: Option<String>,
    pub expression: Option<Expression>,
    #[fhir_serde(rename = "headerField")]
    pub header_field: Option<String>,
    pub hint: Option<String>,
    pub path: Option<String>,
    #[fhir_serde(rename = "sourceId")]
    pub source_id: Option<Id>,
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
pub struct TestScriptScope {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub artifact: Canonical,
    pub conformance: Option<CodeableConcept>,
    pub phase: Option<CodeableConcept>,
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
pub struct TestScriptCommon {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub key: Id,
    pub name: Option<String>,
    pub description: Option<String>,
    pub parameter: Option<Vec<TestScriptCommonParameter>>,
    pub action: Option<Vec<TestScriptCommonAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptCommonAction {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptCommonParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptSetupActionAssert {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub label: Option<String>,
    pub key: Option<Id>,
    #[fhir_serde(rename = "evaluateBasedOn")]
    pub evaluate_based_on: Option<Vec<Id>>,
    pub description: Option<String>,
    pub direction: Option<Code>,
    #[fhir_serde(rename = "compareToSourceId")]
    pub compare_to_source_id: Option<String>,
    #[fhir_serde(rename = "compareToSourceExpression")]
    pub compare_to_source_expression: Option<Expression>,
    #[fhir_serde(rename = "compareToSourcePath")]
    pub compare_to_source_path: Option<String>,
    #[fhir_serde(rename = "contentType")]
    pub content_type: Option<Code>,
    #[fhir_serde(rename = "defaultManualCompletion")]
    pub default_manual_completion: Option<Code>,
    pub expression: Option<Expression>,
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
    pub resource: Option<Uri>,
    pub response: Option<Code>,
    #[fhir_serde(rename = "responseCode")]
    pub response_code: Option<String>,
    #[fhir_serde(rename = "sourceId")]
    pub source_id: Option<Id>,
    #[fhir_serde(rename = "stopTestOnFail")]
    pub stop_test_on_fail: Boolean,
    #[fhir_serde(rename = "validateProfileId")]
    pub validate_profile_id: Option<Id>,
    pub value: Option<String>,
    #[fhir_serde(rename = "warningOnly")]
    pub warning_only: Boolean,
    pub requirement: Option<Vec<TestScriptSetupActionAssertRequirement>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct TestScriptTestSystem {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: PositiveInt,
    pub title: String,
    pub actor: Option<Vec<Canonical>>,
    pub description: Option<Markdown>,
    pub url: Option<Url>,
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
pub struct Transport {
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
    pub instantiates: Option<Canonical>,
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Option<Code>,
    #[fhir_serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub focus: Option<Reference>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub period: Option<Period>,
    #[fhir_serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    #[fhir_serde(rename = "lastModified")]
    pub last_modified: Option<DateTime>,
    pub performer: Option<Vec<Reference>>,
    pub location: Option<Reference>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[fhir_serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
    pub to: Reference,
    pub from: Reference,
    pub reason: Option<CodeableReference>,
    #[fhir_serde(rename = "priorTransport")]
    pub prior_transport: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetScope {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "inclusionCriteria")]
    pub inclusion_criteria: Option<Markdown>,
    #[fhir_serde(rename = "exclusionCriteria")]
    pub exclusion_criteria: Option<Markdown>,
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
    #[fhir_serde(rename = "additionalUse")]
    pub additional_use: Option<Vec<Coding>>,
    pub value: String,
}

/// Choice of types for the versionAlgorithm\[x\] field in ValueSet
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ValueSetVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
    #[fhir_serde(flatten)]
    pub version_algorithm: Option<ValueSetVersionAlgorithm>,
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
    #[fhir_serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
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
    pub compose: Option<ValueSetCompose>,
    pub expansion: Option<ValueSetExpansion>,
    pub scope: Option<ValueSetScope>,
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
pub struct ValueSetExpansionProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub uri: Option<Uri>,
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

/// Choice of types for the value\[x\] field in ValueSetExpansionContainsPropertySubProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum ValueSetExpansionContainsPropertySubPropertyValue {
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
#[fhir_resource(choice_elements = "value")]
pub struct ValueSetExpansionContainsPropertySubProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[fhir_serde(flatten)]
    pub value: Option<ValueSetExpansionContainsPropertySubPropertyValue>,
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
    pub property: Option<Vec<ValueSetExpansionContainsProperty>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ValueSetExpansion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Uri>,
    pub next: Option<Uri>,
    pub timestamp: DateTime,
    pub total: Option<Integer>,
    pub offset: Option<Integer>,
    pub parameter: Option<Vec<ValueSetExpansionParameter>>,
    pub property: Option<Vec<ValueSetExpansionProperty>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
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
    pub property: Option<Vec<String>>,
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
    pub copyright: Option<Markdown>,
}

/// Choice of types for the value\[x\] field in ValueSetExpansionParameter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
pub struct ValueSetExpansionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[fhir_serde(flatten)]
    pub value: Option<ValueSetExpansionParameterValue>,
}

/// Choice of types for the value\[x\] field in ValueSetExpansionContainsProperty
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum ValueSetExpansionContainsPropertyValue {
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
#[fhir_resource(choice_elements = "value")]
pub struct ValueSetExpansionContainsProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[fhir_serde(flatten)]
    pub value: Option<ValueSetExpansionContainsPropertyValue>,
    #[fhir_serde(rename = "subProperty")]
    pub sub_property: Option<Vec<ValueSetExpansionContainsPropertySubProperty>>,
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
    #[fhir_serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[fhir_serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    pub priority: Option<Code>,
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
pub struct VisionPrescriptionLensSpecificationPrism {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub amount: Decimal,
    pub base: Code,
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

/// Choice of types for the value\[x\] field in ViewDefinitionConstant
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "valueInteger64")]
    Integer64(Integer64),
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
#[fhir_resource(choice_elements = "value")]
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
#[fhir_choice_element(base_name = "versionAlgorithm")]
pub enum ViewDefinitionVersionAlgorithm {
    /// Variant accepting the String type.
    #[fhir_serde(rename = "versionAlgorithmString")]
    String(String),
    /// Variant accepting the Coding type.
    #[fhir_serde(rename = "versionAlgorithmCoding")]
    Coding(Coding),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "versionAlgorithm")]
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
pub struct ViewDefinitionSelectColumnTag {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub value: String,
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
pub struct ViewDefinitionWhere {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub description: Option<String>,
}

pub type Base64Binary = Element<std::string::String, Extension>;

pub type Boolean = Element<bool, Extension>;

pub type Canonical = Element<std::string::String, Extension>;

pub type Code = Element<std::string::String, Extension>;

pub type Date = Element<crate::PrecisionDate, Extension>;

pub type DateTime = Element<crate::PrecisionDateTime, Extension>;

pub type Decimal = DecimalElement<Extension>;

pub type Id = Element<std::string::String, Extension>;

pub type Instant = Element<crate::PrecisionInstant, Extension>;

pub type Integer = Element<std::primitive::i32, Extension>;

pub type Integer64 = Element<std::primitive::i64, Extension>;

pub type Markdown = Element<std::string::String, Extension>;

pub type Oid = Element<std::string::String, Extension>;

pub type PositiveInt = Element<std::primitive::i32, Extension>;

pub type String = Element<std::string::String, Extension>;

pub type Time = Element<crate::PrecisionTime, Extension>;

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
#[fhir_choice_element(base_name = "author")]
pub enum AnnotationAuthor {
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "authorReference")]
    Reference(Reference),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "authorString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "author")]
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
    pub size: Option<Integer64>,
    pub hash: Option<Base64Binary>,
    pub title: Option<String>,
    pub creation: Option<DateTime>,
    pub height: Option<PositiveInt>,
    pub width: Option<PositiveInt>,
    pub frames: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    pub pages: Option<PositiveInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AvailabilityNotAvailableTime {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub during: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct AvailabilityAvailableTime {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
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
pub struct Availability {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub period: Option<Period>,
    #[fhir_serde(rename = "availableTime")]
    pub available_time: Option<Vec<AvailabilityAvailableTime>>,
    #[fhir_serde(rename = "notAvailableTime")]
    pub not_available_time: Option<Vec<AvailabilityNotAvailableTime>>,
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

/// Choice of types for the value\[x\] field in DataRequirementValueFilter
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
pub enum DataRequirementValueFilterValue {
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
#[fhir_resource(choice_elements = "value")]
pub struct DataRequirementValueFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    #[fhir_serde(rename = "searchParam")]
    pub search_param: Option<String>,
    pub comparator: Option<Code>,
    #[fhir_serde(flatten)]
    pub value: Option<DataRequirementValueFilterValue>,
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
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
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
pub struct DataRequirementSort {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub direction: Code,
}

/// Choice of types for the subject\[x\] field in DataRequirement
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "subject")]
pub enum DataRequirementSubject {
    /// Variant accepting the CodeableConcept type.
    #[fhir_serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[fhir_serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "subject")]
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
    #[fhir_serde(rename = "valueFilter")]
    pub value_filter: Option<Vec<DataRequirementValueFilter>>,
    pub limit: Option<PositiveInt>,
    pub sort: Option<Vec<DataRequirementSort>>,
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

/// Choice of types for the dose\[x\] field in DosageDoseAndRate
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "dose")]
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
#[fhir_choice_element(base_name = "rate")]
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
#[fhir_resource(choice_elements = "dose,rate")]
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
    #[fhir_serde(rename = "asNeeded")]
    pub as_needed: Option<Boolean>,
    #[fhir_serde(rename = "asNeededFor")]
    pub as_needed_for: Option<Vec<CodeableConcept>>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    #[fhir_serde(rename = "doseAndRate")]
    pub dose_and_rate: Option<Vec<DosageDoseAndRate>>,
    #[fhir_serde(rename = "maxDosePerPeriod")]
    pub max_dose_per_period: Option<Vec<Ratio>>,
    #[fhir_serde(rename = "maxDosePerAdministration")]
    pub max_dose_per_administration: Option<Quantity>,
    #[fhir_serde(rename = "maxDosePerLifetime")]
    pub max_dose_per_lifetime: Option<Quantity>,
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

/// Choice of types for the defaultValue\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "defaultValue")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "defaultValueInteger64")]
    Integer64(Integer64),
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
    /// Variant accepting the Availability type.
    #[fhir_serde(rename = "defaultValueAvailability")]
    Availability(Availability),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "defaultValueExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "defaultValueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "defaultValueMeta")]
    Meta(Meta),
}

/// Choice of types for the fixed\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "fixed")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "fixedInteger64")]
    Integer64(Integer64),
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
    /// Variant accepting the Availability type.
    #[fhir_serde(rename = "fixedAvailability")]
    Availability(Availability),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "fixedExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "fixedDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "fixedMeta")]
    Meta(Meta),
}

/// Choice of types for the pattern\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "pattern")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "patternInteger64")]
    Integer64(Integer64),
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
    /// Variant accepting the Availability type.
    #[fhir_serde(rename = "patternAvailability")]
    Availability(Availability),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "patternExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "patternDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "patternMeta")]
    Meta(Meta),
}

/// Choice of types for the minValue\[x\] field in ElementDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "minValue")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "minValueInteger64")]
    Integer64(Integer64),
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
#[fhir_choice_element(base_name = "maxValue")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "maxValueInteger64")]
    Integer64(Integer64),
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
#[fhir_resource(choice_elements = "defaultValue,fixed,pattern,minValue,maxValue")]
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
    #[fhir_serde(rename = "mustHaveValue")]
    pub must_have_value: Option<Boolean>,
    #[fhir_serde(rename = "valueAlternatives")]
    pub value_alternatives: Option<Vec<Canonical>>,
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
pub struct ElementDefinitionConstraint {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub key: Id,
    pub requirements: Option<Markdown>,
    pub severity: Code,
    pub suppress: Option<Boolean>,
    pub human: String,
    pub expression: Option<String>,
    pub source: Option<Canonical>,
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
pub struct ElementDefinitionSlicing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub discriminator: Option<Vec<ElementDefinitionSlicingDiscriminator>>,
    pub description: Option<String>,
    pub ordered: Option<Boolean>,
    pub rules: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionBinding {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub strength: Code,
    pub description: Option<Markdown>,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub additional: Option<Vec<ElementDefinitionBindingAdditional>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionMapping {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub identity: Id,
    pub language: Option<Code>,
    pub map: String,
    pub comment: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionBindingAdditional {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub key: Option<Id>,
    pub purpose: Code,
    #[fhir_serde(rename = "valueSet")]
    pub value_set: Canonical,
    pub documentation: Option<Markdown>,
    #[fhir_serde(rename = "shortDoco")]
    pub short_doco: Option<String>,
    pub usage: Option<Vec<UsageContext>>,
    pub any: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ElementDefinitionSlicingDiscriminator {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub path: String,
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
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "valueInteger64")]
    Integer64(Integer64),
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
    /// Variant accepting the Availability type.
    #[fhir_serde(rename = "valueAvailability")]
    Availability(Availability),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "valueExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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
    pub name: Option<Code>,
    pub language: Option<Code>,
    pub expression: Option<String>,
    pub reference: Option<Uri>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct ExtendedContactDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub purpose: Option<CodeableConcept>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
    pub organization: Option<Reference>,
    pub period: Option<Period>,
}

/// Choice of types for the value\[x\] field in Extension
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
    /// Variant accepting the Integer64 type.
    #[fhir_serde(rename = "valueInteger64")]
    Integer64(Integer64),
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
    /// Variant accepting the Availability type.
    #[fhir_serde(rename = "valueAvailability")]
    Availability(Availability),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "valueExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
    /// Variant accepting the Dosage type.
    #[fhir_serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[fhir_serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "value")]
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
pub struct MonetaryComponent {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub code: Option<CodeableConcept>,
    pub factor: Option<Decimal>,
    pub amount: Option<Money>,
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

/// Choice of types for the period\[x\] field in ProductShelfLife
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "period")]
pub enum ProductShelfLifePeriod {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "periodDuration")]
    Duration(Duration),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "periodString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "period")]
pub struct ProductShelfLife {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub period: Option<ProductShelfLifePeriod>,
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
    pub classifier: Option<Vec<CodeableConcept>>,
    pub label: Option<String>,
    pub display: Option<String>,
    pub citation: Option<Markdown>,
    pub document: Option<Attachment>,
    pub resource: Option<Canonical>,
    #[fhir_serde(rename = "resourceReference")]
    pub resource_reference: Option<Reference>,
    #[fhir_serde(rename = "publicationStatus")]
    pub publication_status: Option<Code>,
    #[fhir_serde(rename = "publicationDate")]
    pub publication_date: Option<Date>,
}

/// Choice of types for the offset\[x\] field in RelativeTime
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "offset")]
pub enum RelativeTimeOffset {
    /// Variant accepting the Duration type.
    #[fhir_serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[fhir_serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "offset")]
pub struct RelativeTime {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "contextReference")]
    pub context_reference: Option<Reference>,
    #[fhir_serde(rename = "contextDefinition")]
    pub context_definition: Option<Canonical>,
    #[fhir_serde(rename = "contextPath")]
    pub context_path: Option<String>,
    #[fhir_serde(rename = "contextCode")]
    pub context_code: Option<CodeableConcept>,
    #[fhir_serde(flatten)]
    pub offset: Option<RelativeTimeOffset>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct SampledData {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub origin: Quantity,
    pub interval: Option<Decimal>,
    #[fhir_serde(rename = "intervalUnit")]
    pub interval_unit: Code,
    pub factor: Option<Decimal>,
    #[fhir_serde(rename = "lowerLimit")]
    pub lower_limit: Option<Decimal>,
    #[fhir_serde(rename = "upperLimit")]
    pub upper_limit: Option<Decimal>,
    pub dimensions: PositiveInt,
    #[fhir_serde(rename = "codeMap")]
    pub code_map: Option<Canonical>,
    pub offsets: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
pub struct Signature {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Option<Vec<Coding>>,
    pub when: Option<Instant>,
    pub who: Option<Reference>,
    #[fhir_serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    #[fhir_serde(rename = "targetFormat")]
    pub target_format: Option<Code>,
    #[fhir_serde(rename = "sigFormat")]
    pub sig_format: Option<Code>,
    pub data: Option<Base64Binary>,
}

/// Choice of types for the bounds\[x\] field in TimingRepeat
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "bounds")]
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
#[fhir_resource(choice_elements = "bounds")]
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

/// Choice of types for the timing\[x\] field in TriggerDefinition
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "timing")]
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
#[fhir_resource(choice_elements = "timing")]
pub struct TriggerDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "type")]
    pub r#type: Code,
    pub name: Option<String>,
    pub code: Option<CodeableConcept>,
    #[fhir_serde(rename = "subscriptionTopic")]
    pub subscription_topic: Option<Canonical>,
    #[fhir_serde(flatten)]
    pub timing: Option<TriggerDefinitionTiming>,
    pub data: Option<Vec<DataRequirement>>,
    pub condition: Option<Expression>,
}

/// Choice of types for the value\[x\] field in UsageContext
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "value")]
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
#[fhir_resource(choice_elements = "value")]
pub struct UsageContext {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Coding,
    #[fhir_serde(flatten)]
    pub value: Option<UsageContextValue>,
}

/// Choice of types for the address\[x\] field in VirtualServiceDetail
#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath)]
#[fhir_choice_element(base_name = "address")]
pub enum VirtualServiceDetailAddress {
    /// Variant accepting the Url type.
    #[fhir_serde(rename = "addressUrl")]
    Url(Url),
    /// Variant accepting the String type.
    #[fhir_serde(rename = "addressString")]
    String(String),
    /// Variant accepting the ContactPoint type.
    #[fhir_serde(rename = "addressContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the ExtendedContactDetail type.
    #[fhir_serde(rename = "addressExtendedContactDetail")]
    ExtendedContactDetail(ExtendedContactDetail),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, FhirPath, Default)]
#[fhir_resource(choice_elements = "address")]
pub struct VirtualServiceDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[fhir_serde(rename = "channelType")]
    pub channel_type: Option<Coding>,
    #[fhir_serde(flatten)]
    pub address: Option<VirtualServiceDetailAddress>,
    #[fhir_serde(rename = "additionalInfo")]
    pub additional_info: Option<Vec<Url>>,
    #[fhir_serde(rename = "maxParticipants")]
    pub max_participants: Option<PositiveInt>,
    #[fhir_serde(rename = "sessionKey")]
    pub session_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, FhirPath)]
#[serde(tag = "resourceType")]
pub enum Resource {
    Account(Account),
    ActivityDefinition(ActivityDefinition),
    ActorDefinition(ActorDefinition),
    AdministrableProductDefinition(AdministrableProductDefinition),
    AdverseEvent(AdverseEvent),
    AllergyIntolerance(AllergyIntolerance),
    Appointment(Appointment),
    AppointmentResponse(AppointmentResponse),
    ArtifactAssessment(ArtifactAssessment),
    AuditEvent(AuditEvent),
    Basic(Basic),
    Binary(Binary),
    BiologicallyDerivedProduct(BiologicallyDerivedProduct),
    BiologicallyDerivedProductDispense(BiologicallyDerivedProductDispense),
    BodyStructure(BodyStructure),
    Bundle(Bundle),
    CapabilityStatement(CapabilityStatement),
    CarePlan(CarePlan),
    CareTeam(CareTeam),
    ChargeItem(ChargeItem),
    ChargeItemDefinition(ChargeItemDefinition),
    Citation(Citation),
    Claim(Claim),
    ClaimResponse(ClaimResponse),
    ClinicalAssessment(ClinicalAssessment),
    ClinicalUseDefinition(ClinicalUseDefinition),
    CodeSystem(CodeSystem),
    Communication(Communication),
    CommunicationRequest(CommunicationRequest),
    CompartmentDefinition(CompartmentDefinition),
    Composition(Composition),
    ConceptMap(ConceptMap),
    Condition(Condition),
    ConditionDefinition(ConditionDefinition),
    Consent(Consent),
    Contract(Contract),
    Coverage(Coverage),
    CoverageEligibilityRequest(CoverageEligibilityRequest),
    CoverageEligibilityResponse(CoverageEligibilityResponse),
    DetectedIssue(DetectedIssue),
    Device(Device),
    DeviceAlert(DeviceAlert),
    DeviceAssociation(DeviceAssociation),
    DeviceDefinition(DeviceDefinition),
    DeviceDispense(DeviceDispense),
    DeviceMetric(DeviceMetric),
    DeviceRequest(DeviceRequest),
    DeviceUsage(DeviceUsage),
    DiagnosticReport(DiagnosticReport),
    DocumentReference(DocumentReference),
    Encounter(Encounter),
    EncounterHistory(EncounterHistory),
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
    FormularyItem(FormularyItem),
    GenomicStudy(GenomicStudy),
    Goal(Goal),
    GraphDefinition(GraphDefinition),
    Group(Group),
    GuidanceResponse(GuidanceResponse),
    HealthcareService(HealthcareService),
    ImagingSelection(ImagingSelection),
    ImagingStudy(ImagingStudy),
    Immunization(Immunization),
    ImmunizationEvaluation(ImmunizationEvaluation),
    ImmunizationRecommendation(ImmunizationRecommendation),
    ImplementationGuide(ImplementationGuide),
    Ingredient(Ingredient),
    InsurancePlan(InsurancePlan),
    InsuranceProduct(InsuranceProduct),
    InventoryItem(InventoryItem),
    InventoryReport(InventoryReport),
    Invoice(Invoice),
    Library(Library),
    Linkage(Linkage),
    List(List),
    Location(Location),
    ManufacturedItemDefinition(ManufacturedItemDefinition),
    Measure(Measure),
    MeasureReport(MeasureReport),
    Medication(Medication),
    MedicationAdministration(MedicationAdministration),
    MedicationDispense(MedicationDispense),
    MedicationKnowledge(MedicationKnowledge),
    MedicationRequest(MedicationRequest),
    MedicationStatement(MedicationStatement),
    MedicinalProductDefinition(MedicinalProductDefinition),
    MessageDefinition(MessageDefinition),
    MessageHeader(MessageHeader),
    MolecularDefinition(MolecularDefinition),
    NamingSystem(NamingSystem),
    NutritionIntake(NutritionIntake),
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
    Permission(Permission),
    Person(Person),
    PersonalRelationship(PersonalRelationship),
    PlanDefinition(PlanDefinition),
    Practitioner(Practitioner),
    PractitionerRole(PractitionerRole),
    Procedure(Procedure),
    Provenance(Provenance),
    Questionnaire(Questionnaire),
    QuestionnaireResponse(QuestionnaireResponse),
    RegulatedAuthorization(RegulatedAuthorization),
    RelatedPerson(RelatedPerson),
    RequestOrchestration(RequestOrchestration),
    Requirements(Requirements),
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
    SubstanceNucleicAcid(SubstanceNucleicAcid),
    SubstancePolymer(SubstancePolymer),
    SubstanceProtein(SubstanceProtein),
    SubstanceReferenceInformation(SubstanceReferenceInformation),
    SubstanceSourceMaterial(SubstanceSourceMaterial),
    SupplyDelivery(SupplyDelivery),
    SupplyRequest(SupplyRequest),
    Task(Task),
    TerminologyCapabilities(TerminologyCapabilities),
    TestPlan(TestPlan),
    TestReport(TestReport),
    TestScript(TestScript),
    Transport(Transport),
    ValueSet(ValueSet),
    VerificationResult(VerificationResult),
    VisionPrescription(VisionPrescription),
    ViewDefinition(ViewDefinition),
}

// --- From<T> Implementations for Element<T, Extension> ---
impl From<bool> for Element<bool, Extension> {
    fn from(value: bool) -> Self {
        Self {
            value: Some(value),
            ..Default::default()
        }
    }
}
impl From<std::primitive::i32> for Element<std::primitive::i32, Extension> {
    fn from(value: std::primitive::i32) -> Self {
        Self {
            value: Some(value),
            ..Default::default()
        }
    }
}
impl From<std::string::String> for Element<std::string::String, Extension> {
    fn from(value: std::string::String) -> Self {
        Self {
            value: Some(value),
            ..Default::default()
        }
    }
}
// --- End From<T> Implementations ---

// --- Type Hierarchy Module ---
/// Type hierarchy information extracted from FHIR specifications
pub mod type_hierarchy {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    /// Maps FHIR type names to their parent types
    static TYPE_PARENTS: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

    fn get_type_parents() -> &'static HashMap<&'static str, &'static str> {
        TYPE_PARENTS.get_or_init(|| {
            let mut m = HashMap::new();
            m.insert("Account", "DomainResource");
            m.insert("ActivityDefinition", "DomainResource");
            m.insert("ActorDefinition", "DomainResource");
            m.insert("Address", "DataType");
            m.insert("AdministrableProductDefinition", "DomainResource");
            m.insert("AdverseEvent", "DomainResource");
            m.insert("Age", "Quantity");
            m.insert("AllergyIntolerance", "DomainResource");
            m.insert("Annotation", "DataType");
            m.insert("Appointment", "DomainResource");
            m.insert("AppointmentResponse", "DomainResource");
            m.insert("ArtifactAssessment", "DomainResource");
            m.insert("Attachment", "DataType");
            m.insert("AuditEvent", "DomainResource");
            m.insert("Availability", "DataType");
            m.insert("Basic", "DomainResource");
            m.insert("Binary", "Resource");
            m.insert("BiologicallyDerivedProduct", "DomainResource");
            m.insert("BiologicallyDerivedProductDispense", "DomainResource");
            m.insert("BodyStructure", "DomainResource");
            m.insert("Bundle", "Resource");
            m.insert("CapabilityStatement", "DomainResource");
            m.insert("CarePlan", "DomainResource");
            m.insert("CareTeam", "DomainResource");
            m.insert("ChargeItem", "DomainResource");
            m.insert("ChargeItemDefinition", "DomainResource");
            m.insert("Citation", "DomainResource");
            m.insert("Claim", "DomainResource");
            m.insert("ClaimResponse", "DomainResource");
            m.insert("ClinicalAssessment", "DomainResource");
            m.insert("ClinicalUseDefinition", "DomainResource");
            m.insert("CodeSystem", "DomainResource");
            m.insert("CodeableConcept", "DataType");
            m.insert("CodeableReference", "DataType");
            m.insert("Coding", "DataType");
            m.insert("Communication", "DomainResource");
            m.insert("CommunicationRequest", "DomainResource");
            m.insert("CompartmentDefinition", "DomainResource");
            m.insert("Composition", "DomainResource");
            m.insert("ConceptMap", "DomainResource");
            m.insert("Condition", "DomainResource");
            m.insert("ConditionDefinition", "DomainResource");
            m.insert("Consent", "DomainResource");
            m.insert("ContactDetail", "DataType");
            m.insert("ContactPoint", "DataType");
            m.insert("Contract", "DomainResource");
            m.insert("Contributor", "DataType");
            m.insert("Count", "Quantity");
            m.insert("Coverage", "DomainResource");
            m.insert("CoverageEligibilityRequest", "DomainResource");
            m.insert("CoverageEligibilityResponse", "DomainResource");
            m.insert("DataRequirement", "DataType");
            m.insert("DetectedIssue", "DomainResource");
            m.insert("Device", "DomainResource");
            m.insert("DeviceAlert", "DomainResource");
            m.insert("DeviceAssociation", "DomainResource");
            m.insert("DeviceDefinition", "DomainResource");
            m.insert("DeviceDispense", "DomainResource");
            m.insert("DeviceMetric", "DomainResource");
            m.insert("DeviceRequest", "DomainResource");
            m.insert("DeviceUsage", "DomainResource");
            m.insert("DiagnosticReport", "DomainResource");
            m.insert("Distance", "Quantity");
            m.insert("DocumentReference", "DomainResource");
            m.insert("Dosage", "BackboneType");
            m.insert("Duration", "Quantity");
            m.insert("ElementDefinition", "BackboneType");
            m.insert("Encounter", "DomainResource");
            m.insert("EncounterHistory", "DomainResource");
            m.insert("Endpoint", "DomainResource");
            m.insert("EnrollmentRequest", "DomainResource");
            m.insert("EnrollmentResponse", "DomainResource");
            m.insert("EpisodeOfCare", "DomainResource");
            m.insert("EventDefinition", "DomainResource");
            m.insert("Evidence", "DomainResource");
            m.insert("EvidenceVariable", "DomainResource");
            m.insert("ExampleScenario", "DomainResource");
            m.insert("ExplanationOfBenefit", "DomainResource");
            m.insert("Expression", "DataType");
            m.insert("ExtendedContactDetail", "DataType");
            m.insert("Extension", "DataType");
            m.insert("FamilyMemberHistory", "DomainResource");
            m.insert("Flag", "DomainResource");
            m.insert("FormularyItem", "DomainResource");
            m.insert("GenomicStudy", "DomainResource");
            m.insert("Goal", "DomainResource");
            m.insert("GraphDefinition", "DomainResource");
            m.insert("Group", "DomainResource");
            m.insert("GuidanceResponse", "DomainResource");
            m.insert("HealthcareService", "DomainResource");
            m.insert("HumanName", "DataType");
            m.insert("Identifier", "DataType");
            m.insert("ImagingSelection", "DomainResource");
            m.insert("ImagingStudy", "DomainResource");
            m.insert("Immunization", "DomainResource");
            m.insert("ImmunizationEvaluation", "DomainResource");
            m.insert("ImmunizationRecommendation", "DomainResource");
            m.insert("ImplementationGuide", "DomainResource");
            m.insert("Ingredient", "DomainResource");
            m.insert("InsurancePlan", "DomainResource");
            m.insert("InsuranceProduct", "DomainResource");
            m.insert("InventoryItem", "DomainResource");
            m.insert("InventoryReport", "DomainResource");
            m.insert("Invoice", "DomainResource");
            m.insert("Library", "DomainResource");
            m.insert("Linkage", "DomainResource");
            m.insert("List", "DomainResource");
            m.insert("Location", "DomainResource");
            m.insert("ManufacturedItemDefinition", "DomainResource");
            m.insert("MarketingStatus", "BackboneType");
            m.insert("Measure", "DomainResource");
            m.insert("MeasureReport", "DomainResource");
            m.insert("Medication", "DomainResource");
            m.insert("MedicationAdministration", "DomainResource");
            m.insert("MedicationDispense", "DomainResource");
            m.insert("MedicationKnowledge", "DomainResource");
            m.insert("MedicationRequest", "DomainResource");
            m.insert("MedicationStatement", "DomainResource");
            m.insert("MedicinalProductDefinition", "DomainResource");
            m.insert("MessageDefinition", "DomainResource");
            m.insert("MessageHeader", "DomainResource");
            m.insert("Meta", "DataType");
            m.insert("MolecularDefinition", "DomainResource");
            m.insert("MonetaryComponent", "DataType");
            m.insert("Money", "DataType");
            m.insert("NamingSystem", "DomainResource");
            m.insert("Narrative", "DataType");
            m.insert("NutritionIntake", "DomainResource");
            m.insert("NutritionOrder", "DomainResource");
            m.insert("NutritionProduct", "DomainResource");
            m.insert("Observation", "DomainResource");
            m.insert("ObservationDefinition", "DomainResource");
            m.insert("OperationDefinition", "DomainResource");
            m.insert("OperationOutcome", "DomainResource");
            m.insert("Organization", "DomainResource");
            m.insert("OrganizationAffiliation", "DomainResource");
            m.insert("PackagedProductDefinition", "DomainResource");
            m.insert("ParameterDefinition", "DataType");
            m.insert("Parameters", "Resource");
            m.insert("Patient", "DomainResource");
            m.insert("PaymentNotice", "DomainResource");
            m.insert("PaymentReconciliation", "DomainResource");
            m.insert("Period", "DataType");
            m.insert("Permission", "DomainResource");
            m.insert("Person", "DomainResource");
            m.insert("PersonalRelationship", "DomainResource");
            m.insert("PlanDefinition", "DomainResource");
            m.insert("Practitioner", "DomainResource");
            m.insert("PractitionerRole", "DomainResource");
            m.insert("Procedure", "DomainResource");
            m.insert("ProductShelfLife", "BackboneType");
            m.insert("Provenance", "DomainResource");
            m.insert("Quantity", "DataType");
            m.insert("Questionnaire", "DomainResource");
            m.insert("QuestionnaireResponse", "DomainResource");
            m.insert("Range", "DataType");
            m.insert("Ratio", "DataType");
            m.insert("RatioRange", "DataType");
            m.insert("Reference", "DataType");
            m.insert("RegulatedAuthorization", "DomainResource");
            m.insert("RelatedArtifact", "DataType");
            m.insert("RelatedPerson", "DomainResource");
            m.insert("RelativeTime", "BackboneType");
            m.insert("RequestOrchestration", "DomainResource");
            m.insert("Requirements", "DomainResource");
            m.insert("ResearchStudy", "DomainResource");
            m.insert("ResearchSubject", "DomainResource");
            m.insert("RiskAssessment", "DomainResource");
            m.insert("SampledData", "DataType");
            m.insert("Schedule", "DomainResource");
            m.insert("SearchParameter", "DomainResource");
            m.insert("ServiceRequest", "DomainResource");
            m.insert("Signature", "DataType");
            m.insert("Slot", "DomainResource");
            m.insert("Specimen", "DomainResource");
            m.insert("SpecimenDefinition", "DomainResource");
            m.insert("StructureDefinition", "DomainResource");
            m.insert("StructureMap", "DomainResource");
            m.insert("Subscription", "DomainResource");
            m.insert("SubscriptionStatus", "DomainResource");
            m.insert("SubscriptionTopic", "DomainResource");
            m.insert("Substance", "DomainResource");
            m.insert("SubstanceDefinition", "DomainResource");
            m.insert("SubstanceNucleicAcid", "DomainResource");
            m.insert("SubstancePolymer", "DomainResource");
            m.insert("SubstanceProtein", "DomainResource");
            m.insert("SubstanceReferenceInformation", "DomainResource");
            m.insert("SubstanceSourceMaterial", "DomainResource");
            m.insert("SupplyDelivery", "DomainResource");
            m.insert("SupplyRequest", "DomainResource");
            m.insert("Task", "DomainResource");
            m.insert("TerminologyCapabilities", "DomainResource");
            m.insert("TestPlan", "DomainResource");
            m.insert("TestReport", "DomainResource");
            m.insert("TestScript", "DomainResource");
            m.insert("Timing", "BackboneType");
            m.insert("Transport", "DomainResource");
            m.insert("TriggerDefinition", "DataType");
            m.insert("UsageContext", "DataType");
            m.insert("ValueSet", "DomainResource");
            m.insert("VerificationResult", "DomainResource");
            m.insert("ViewDefinition", "CanonicalResource");
            m.insert("VirtualServiceDetail", "DataType");
            m.insert("VisionPrescription", "DomainResource");
            m.insert("base64Binary", "PrimitiveType");
            m.insert("boolean", "PrimitiveType");
            m.insert("canonical", "uri");
            m.insert("code", "string");
            m.insert("date", "PrimitiveType");
            m.insert("dateTime", "PrimitiveType");
            m.insert("decimal", "PrimitiveType");
            m.insert("id", "string");
            m.insert("instant", "PrimitiveType");
            m.insert("integer", "PrimitiveType");
            m.insert("integer64", "PrimitiveType");
            m.insert("markdown", "string");
            m.insert("oid", "uri");
            m.insert("positiveInt", "integer");
            m.insert("string", "PrimitiveType");
            m.insert("time", "PrimitiveType");
            m.insert("unsignedInt", "integer");
            m.insert("uri", "PrimitiveType");
            m.insert("url", "uri");
            m.insert("uuid", "uri");
            m.insert("xhtml", "Element");
            m
        })
    }

    /// Checks if a type is a subtype of another type
    pub fn is_subtype_of(child: &str, parent: &str) -> bool {
        // Direct match
        if child.eq_ignore_ascii_case(parent) {
            return true;
        }

        // Walk up the type hierarchy
        let mut current = child;
        while let Some(&parent_type) = get_type_parents().get(current) {
            if parent_type.eq_ignore_ascii_case(parent) {
                return true;
            }
            current = parent_type;
        }
        false
    }

    /// Gets the parent type of a given type
    pub fn get_parent_type(type_name: &str) -> Option<&'static str> {
        get_type_parents().get(type_name).copied()
    }

    /// Gets all subtypes of a given parent type
    pub fn get_subtypes(parent: &str) -> Vec<&'static str> {
        get_type_parents()
            .iter()
            .filter_map(|(child, p)| {
                if p.eq_ignore_ascii_case(parent) {
                    Some(*child)
                } else {
                    None
                }
            })
            .collect()
    }
}

// --- Complex Types Provider ---
/// Marker struct for complex type information
pub struct ComplexTypes;

impl crate::FhirComplexTypeProvider for ComplexTypes {
    fn get_complex_type_names() -> Vec<&'static str> {
        vec![
            "Address",
            "Age",
            "Annotation",
            "Attachment",
            "Availability",
            "CodeableConcept",
            "CodeableReference",
            "Coding",
            "ContactDetail",
            "ContactPoint",
            "Contributor",
            "Count",
            "DataRequirement",
            "Distance",
            "Dosage",
            "Duration",
            "ElementDefinition",
            "Expression",
            "ExtendedContactDetail",
            "Extension",
            "HumanName",
            "Identifier",
            "MarketingStatus",
            "Meta",
            "MonetaryComponent",
            "Money",
            "Narrative",
            "ParameterDefinition",
            "Period",
            "ProductShelfLife",
            "Quantity",
            "Range",
            "Ratio",
            "RatioRange",
            "Reference",
            "RelatedArtifact",
            "RelativeTime",
            "SampledData",
            "Signature",
            "Timing",
            "TriggerDefinition",
            "UsageContext",
            "VirtualServiceDetail",
        ]
    }
}
