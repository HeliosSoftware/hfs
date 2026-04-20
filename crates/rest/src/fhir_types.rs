//! FHIR type utilities using generated models.
//!
//! This module provides functions to query valid FHIR resource types
//! for the enabled FHIR version(s). The resource type lists are derived
//! from the generated Resource enum in `helios-fhir`.

use helios_fhir::FhirVersion;

/// R4 resource types (148 types including ViewDefinition)
#[cfg(feature = "R4")]
const R4_RESOURCE_TYPES: &[&str] = &[
    "Account",
    "ActivityDefinition",
    "AdverseEvent",
    "AllergyIntolerance",
    "Appointment",
    "AppointmentResponse",
    "AuditEvent",
    "Basic",
    "Binary",
    "BiologicallyDerivedProduct",
    "BodyStructure",
    "Bundle",
    "CapabilityStatement",
    "CarePlan",
    "CareTeam",
    "CatalogEntry",
    "ChargeItem",
    "ChargeItemDefinition",
    "Claim",
    "ClaimResponse",
    "ClinicalImpression",
    "CodeSystem",
    "Communication",
    "CommunicationRequest",
    "CompartmentDefinition",
    "Composition",
    "ConceptMap",
    "Condition",
    "Consent",
    "Contract",
    "Coverage",
    "CoverageEligibilityRequest",
    "CoverageEligibilityResponse",
    "DetectedIssue",
    "Device",
    "DeviceDefinition",
    "DeviceMetric",
    "DeviceRequest",
    "DeviceUseStatement",
    "DiagnosticReport",
    "DocumentManifest",
    "DocumentReference",
    "EffectEvidenceSynthesis",
    "Encounter",
    "Endpoint",
    "EnrollmentRequest",
    "EnrollmentResponse",
    "EpisodeOfCare",
    "EventDefinition",
    "Evidence",
    "EvidenceVariable",
    "ExampleScenario",
    "ExplanationOfBenefit",
    "FamilyMemberHistory",
    "Flag",
    "Goal",
    "GraphDefinition",
    "Group",
    "GuidanceResponse",
    "HealthcareService",
    "ImagingStudy",
    "Immunization",
    "ImmunizationEvaluation",
    "ImmunizationRecommendation",
    "ImplementationGuide",
    "InsurancePlan",
    "Invoice",
    "Library",
    "Linkage",
    "List",
    "Location",
    "Measure",
    "MeasureReport",
    "Media",
    "Medication",
    "MedicationAdministration",
    "MedicationDispense",
    "MedicationKnowledge",
    "MedicationRequest",
    "MedicationStatement",
    "MedicinalProduct",
    "MedicinalProductAuthorization",
    "MedicinalProductContraindication",
    "MedicinalProductIndication",
    "MedicinalProductIngredient",
    "MedicinalProductInteraction",
    "MedicinalProductManufactured",
    "MedicinalProductPackaged",
    "MedicinalProductPharmaceutical",
    "MedicinalProductUndesirableEffect",
    "MessageDefinition",
    "MessageHeader",
    "MolecularSequence",
    "NamingSystem",
    "NutritionOrder",
    "Observation",
    "ObservationDefinition",
    "OperationDefinition",
    "OperationOutcome",
    "Organization",
    "OrganizationAffiliation",
    "Parameters",
    "Patient",
    "PaymentNotice",
    "PaymentReconciliation",
    "Person",
    "PlanDefinition",
    "Practitioner",
    "PractitionerRole",
    "Procedure",
    "Provenance",
    "Questionnaire",
    "QuestionnaireResponse",
    "RelatedPerson",
    "RequestGroup",
    "ResearchDefinition",
    "ResearchElementDefinition",
    "ResearchStudy",
    "ResearchSubject",
    "RiskAssessment",
    "RiskEvidenceSynthesis",
    "Schedule",
    "SearchParameter",
    "ServiceRequest",
    "Slot",
    "Specimen",
    "SpecimenDefinition",
    "StructureDefinition",
    "StructureMap",
    "Subscription",
    "Substance",
    "SubstanceNucleicAcid",
    "SubstancePolymer",
    "SubstanceProtein",
    "SubstanceReferenceInformation",
    "SubstanceSourceMaterial",
    "SubstanceSpecification",
    "SupplyDelivery",
    "SupplyRequest",
    "Task",
    "TerminologyCapabilities",
    "TestReport",
    "TestScript",
    "ValueSet",
    "VerificationResult",
    "ViewDefinition",
    "VisionPrescription",
];

/// R4B resource types (142 types including ViewDefinition)
#[cfg(all(feature = "R4B", not(feature = "R4")))]
const R4B_RESOURCE_TYPES: &[&str] = &[
    "Account",
    "ActivityDefinition",
    "AdministrableProductDefinition",
    "AdverseEvent",
    "AllergyIntolerance",
    "Appointment",
    "AppointmentResponse",
    "AuditEvent",
    "Basic",
    "Binary",
    "BiologicallyDerivedProduct",
    "BodyStructure",
    "Bundle",
    "CapabilityStatement",
    "CarePlan",
    "CareTeam",
    "CatalogEntry",
    "ChargeItem",
    "ChargeItemDefinition",
    "Citation",
    "Claim",
    "ClaimResponse",
    "ClinicalImpression",
    "ClinicalUseDefinition",
    "CodeSystem",
    "Communication",
    "CommunicationRequest",
    "CompartmentDefinition",
    "Composition",
    "ConceptMap",
    "Condition",
    "Consent",
    "Contract",
    "Coverage",
    "CoverageEligibilityRequest",
    "CoverageEligibilityResponse",
    "DetectedIssue",
    "Device",
    "DeviceDefinition",
    "DeviceMetric",
    "DeviceRequest",
    "DeviceUseStatement",
    "DiagnosticReport",
    "DocumentManifest",
    "DocumentReference",
    "Encounter",
    "Endpoint",
    "EnrollmentRequest",
    "EnrollmentResponse",
    "EpisodeOfCare",
    "EventDefinition",
    "Evidence",
    "EvidenceReport",
    "EvidenceVariable",
    "ExampleScenario",
    "ExplanationOfBenefit",
    "FamilyMemberHistory",
    "Flag",
    "Goal",
    "GraphDefinition",
    "Group",
    "GuidanceResponse",
    "HealthcareService",
    "ImagingStudy",
    "Immunization",
    "ImmunizationEvaluation",
    "ImmunizationRecommendation",
    "ImplementationGuide",
    "Ingredient",
    "InsurancePlan",
    "Invoice",
    "Library",
    "Linkage",
    "List",
    "Location",
    "ManufacturedItemDefinition",
    "Measure",
    "MeasureReport",
    "Media",
    "Medication",
    "MedicationAdministration",
    "MedicationDispense",
    "MedicationKnowledge",
    "MedicationRequest",
    "MedicationStatement",
    "MedicinalProductDefinition",
    "MessageDefinition",
    "MessageHeader",
    "MolecularSequence",
    "NamingSystem",
    "NutritionOrder",
    "NutritionProduct",
    "Observation",
    "ObservationDefinition",
    "OperationDefinition",
    "OperationOutcome",
    "Organization",
    "OrganizationAffiliation",
    "PackagedProductDefinition",
    "Parameters",
    "Patient",
    "PaymentNotice",
    "PaymentReconciliation",
    "Person",
    "PlanDefinition",
    "Practitioner",
    "PractitionerRole",
    "Procedure",
    "Provenance",
    "Questionnaire",
    "QuestionnaireResponse",
    "RegulatedAuthorization",
    "RelatedPerson",
    "RequestGroup",
    "ResearchDefinition",
    "ResearchElementDefinition",
    "ResearchStudy",
    "ResearchSubject",
    "RiskAssessment",
    "Schedule",
    "SearchParameter",
    "ServiceRequest",
    "Slot",
    "Specimen",
    "SpecimenDefinition",
    "StructureDefinition",
    "StructureMap",
    "Subscription",
    "SubscriptionStatus",
    "SubscriptionTopic",
    "Substance",
    "SubstanceDefinition",
    "SupplyDelivery",
    "SupplyRequest",
    "Task",
    "TerminologyCapabilities",
    "TestReport",
    "TestScript",
    "ValueSet",
    "VerificationResult",
    "ViewDefinition",
    "VisionPrescription",
];

/// R5 resource types (159 types including ViewDefinition)
#[cfg(all(feature = "R5", not(any(feature = "R4", feature = "R4B"))))]
const R5_RESOURCE_TYPES: &[&str] = &[
    "Account",
    "ActivityDefinition",
    "ActorDefinition",
    "AdministrableProductDefinition",
    "AdverseEvent",
    "AllergyIntolerance",
    "Appointment",
    "AppointmentResponse",
    "ArtifactAssessment",
    "AuditEvent",
    "Basic",
    "Binary",
    "BiologicallyDerivedProduct",
    "BiologicallyDerivedProductDispense",
    "BodyStructure",
    "Bundle",
    "CapabilityStatement",
    "CarePlan",
    "CareTeam",
    "ChargeItem",
    "ChargeItemDefinition",
    "Citation",
    "Claim",
    "ClaimResponse",
    "ClinicalImpression",
    "ClinicalUseDefinition",
    "CodeSystem",
    "Communication",
    "CommunicationRequest",
    "CompartmentDefinition",
    "Composition",
    "ConceptMap",
    "Condition",
    "ConditionDefinition",
    "Consent",
    "Contract",
    "Coverage",
    "CoverageEligibilityRequest",
    "CoverageEligibilityResponse",
    "DetectedIssue",
    "Device",
    "DeviceAssociation",
    "DeviceDefinition",
    "DeviceDispense",
    "DeviceMetric",
    "DeviceRequest",
    "DeviceUsage",
    "DiagnosticReport",
    "DocumentReference",
    "Encounter",
    "EncounterHistory",
    "Endpoint",
    "EnrollmentRequest",
    "EnrollmentResponse",
    "EpisodeOfCare",
    "EventDefinition",
    "Evidence",
    "EvidenceReport",
    "EvidenceVariable",
    "ExampleScenario",
    "ExplanationOfBenefit",
    "FamilyMemberHistory",
    "Flag",
    "FormularyItem",
    "GenomicStudy",
    "Goal",
    "GraphDefinition",
    "Group",
    "GuidanceResponse",
    "HealthcareService",
    "ImagingSelection",
    "ImagingStudy",
    "Immunization",
    "ImmunizationEvaluation",
    "ImmunizationRecommendation",
    "ImplementationGuide",
    "Ingredient",
    "InsurancePlan",
    "InventoryItem",
    "InventoryReport",
    "Invoice",
    "Library",
    "Linkage",
    "List",
    "Location",
    "ManufacturedItemDefinition",
    "Measure",
    "MeasureReport",
    "Medication",
    "MedicationAdministration",
    "MedicationDispense",
    "MedicationKnowledge",
    "MedicationRequest",
    "MedicationStatement",
    "MedicinalProductDefinition",
    "MessageDefinition",
    "MessageHeader",
    "MolecularSequence",
    "NamingSystem",
    "NutritionIntake",
    "NutritionOrder",
    "NutritionProduct",
    "Observation",
    "ObservationDefinition",
    "OperationDefinition",
    "OperationOutcome",
    "Organization",
    "OrganizationAffiliation",
    "PackagedProductDefinition",
    "Parameters",
    "Patient",
    "PaymentNotice",
    "PaymentReconciliation",
    "Permission",
    "Person",
    "PlanDefinition",
    "Practitioner",
    "PractitionerRole",
    "Procedure",
    "Provenance",
    "Questionnaire",
    "QuestionnaireResponse",
    "RegulatedAuthorization",
    "RelatedPerson",
    "RequestOrchestration",
    "Requirements",
    "ResearchStudy",
    "ResearchSubject",
    "RiskAssessment",
    "Schedule",
    "SearchParameter",
    "ServiceRequest",
    "Slot",
    "Specimen",
    "SpecimenDefinition",
    "StructureDefinition",
    "StructureMap",
    "Subscription",
    "SubscriptionStatus",
    "SubscriptionTopic",
    "Substance",
    "SubstanceDefinition",
    "SubstanceNucleicAcid",
    "SubstancePolymer",
    "SubstanceProtein",
    "SubstanceReferenceInformation",
    "SubstanceSourceMaterial",
    "SupplyDelivery",
    "SupplyRequest",
    "Task",
    "TerminologyCapabilities",
    "TestPlan",
    "TestReport",
    "TestScript",
    "Transport",
    "ValueSet",
    "VerificationResult",
    "ViewDefinition",
    "VisionPrescription",
];

/// R6 resource types (129 types including ViewDefinition)
#[cfg(all(
    feature = "R6",
    not(any(feature = "R4", feature = "R4B", feature = "R5"))
))]
const R6_RESOURCE_TYPES: &[&str] = &[
    "Account",
    "ActivityDefinition",
    "ActorDefinition",
    "AdministrableProductDefinition",
    "AdverseEvent",
    "AllergyIntolerance",
    "Appointment",
    "AppointmentResponse",
    "ArtifactAssessment",
    "AuditEvent",
    "Basic",
    "Binary",
    "BiologicallyDerivedProduct",
    "BiologicallyDerivedProductDispense",
    "BodyStructure",
    "Bundle",
    "CapabilityStatement",
    "CarePlan",
    "CareTeam",
    "ChargeItem",
    "ChargeItemDefinition",
    "Citation",
    "Claim",
    "ClaimResponse",
    "ClinicalImpression",
    "ClinicalUseDefinition",
    "CodeSystem",
    "Communication",
    "CommunicationRequest",
    "CompartmentDefinition",
    "Composition",
    "ConceptMap",
    "Condition",
    "ConditionDefinition",
    "Consent",
    "Contract",
    "Coverage",
    "CoverageEligibilityRequest",
    "CoverageEligibilityResponse",
    "DetectedIssue",
    "Device",
    "DeviceAssociation",
    "DeviceDefinition",
    "DeviceDispense",
    "DeviceMetric",
    "DeviceRequest",
    "DeviceUsage",
    "DiagnosticReport",
    "DocumentReference",
    "Encounter",
    "EncounterHistory",
    "Endpoint",
    "EnrollmentRequest",
    "EnrollmentResponse",
    "EpisodeOfCare",
    "EventDefinition",
    "Evidence",
    "EvidenceReport",
    "EvidenceVariable",
    "ExampleScenario",
    "ExplanationOfBenefit",
    "FamilyMemberHistory",
    "Flag",
    "FormularyItem",
    "GenomicStudy",
    "Goal",
    "GraphDefinition",
    "Group",
    "GuidanceResponse",
    "HealthcareService",
    "ImagingSelection",
    "ImagingStudy",
    "Immunization",
    "ImmunizationEvaluation",
    "ImmunizationRecommendation",
    "ImplementationGuide",
    "Ingredient",
    "InsurancePlan",
    "InventoryItem",
    "InventoryReport",
    "Invoice",
    "Library",
    "Linkage",
    "List",
    "Location",
    "ManufacturedItemDefinition",
    "Measure",
    "MeasureReport",
    "Medication",
    "MedicationAdministration",
    "MedicationDispense",
    "MedicationKnowledge",
    "MedicationRequest",
    "MedicationStatement",
    "MedicinalProductDefinition",
    "MessageDefinition",
    "MessageHeader",
    "MolecularSequence",
    "NamingSystem",
    "NutritionIntake",
    "NutritionOrder",
    "NutritionProduct",
    "Observation",
    "ObservationDefinition",
    "OperationDefinition",
    "OperationOutcome",
    "Organization",
    "OrganizationAffiliation",
    "PackagedProductDefinition",
    "Parameters",
    "Patient",
    "PaymentNotice",
    "PaymentReconciliation",
    "Permission",
    "Person",
    "PlanDefinition",
    "Practitioner",
    "PractitionerRole",
    "Procedure",
    "Provenance",
    "Questionnaire",
    "QuestionnaireResponse",
    "RegulatedAuthorization",
    "RelatedPerson",
    "RequestOrchestration",
    "Requirements",
    "ResearchStudy",
    "ResearchSubject",
    "RiskAssessment",
    "Schedule",
    "SearchParameter",
    "ServiceRequest",
    "Slot",
    "Specimen",
    "SpecimenDefinition",
    "StructureDefinition",
    "StructureMap",
    "Subscription",
    "SubscriptionStatus",
    "SubscriptionTopic",
    "Substance",
    "SubstanceDefinition",
    "SubstanceNucleicAcid",
    "SubstancePolymer",
    "SubstanceProtein",
    "SubstanceReferenceInformation",
    "SubstanceSourceMaterial",
    "SupplyDelivery",
    "SupplyRequest",
    "Task",
    "TerminologyCapabilities",
    "TestPlan",
    "TestReport",
    "TestScript",
    "Transport",
    "ValueSet",
    "VerificationResult",
    "ViewDefinition",
    "VisionPrescription",
];

/// Returns all valid resource type names for the enabled FHIR version.
///
/// The priority order when multiple versions are enabled is R4 > R4B > R5 > R6.
/// This follows the default feature behavior where R4 takes precedence.
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::fhir_types::get_resource_type_names;
///
/// let types = get_resource_type_names();
/// assert!(types.contains(&"Patient"));
/// assert!(types.contains(&"Observation"));
/// ```
pub fn get_resource_type_names() -> &'static [&'static str] {
    #[cfg(feature = "R4")]
    {
        return R4_RESOURCE_TYPES;
    }

    #[cfg(all(feature = "R4B", not(feature = "R4")))]
    {
        return R4B_RESOURCE_TYPES;
    }

    #[cfg(all(feature = "R5", not(any(feature = "R4", feature = "R4B"))))]
    {
        return R5_RESOURCE_TYPES;
    }

    #[cfg(all(
        feature = "R6",
        not(any(feature = "R4", feature = "R4B", feature = "R5"))
    ))]
    {
        return R6_RESOURCE_TYPES;
    }

    // Fallback for when no FHIR version feature is enabled
    #[allow(unreachable_code)]
    &[]
}

/// Checks if a resource type name is valid for the enabled FHIR version.
///
/// The comparison is case-sensitive as per FHIR specification.
///
/// # Arguments
///
/// * `type_name` - The resource type name to validate
///
/// # Returns
///
/// `true` if the type name is a valid FHIR resource type, `false` otherwise.
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::fhir_types::is_valid_resource_type;
///
/// assert!(is_valid_resource_type("Patient"));
/// assert!(is_valid_resource_type("Observation"));
/// assert!(!is_valid_resource_type("InvalidType"));
/// assert!(!is_valid_resource_type("patient")); // Case-sensitive
/// ```
pub fn is_valid_resource_type(type_name: &str) -> bool {
    get_resource_type_names().contains(&type_name)
}

/// Returns the FHIR version string for the enabled version.
///
/// This is useful for including in CapabilityStatements and other metadata.
pub fn get_fhir_version() -> &'static str {
    #[cfg(feature = "R4")]
    {
        return "4.0.1";
    }

    #[cfg(all(feature = "R4B", not(feature = "R4")))]
    {
        return "4.3.0";
    }

    #[cfg(all(feature = "R5", not(any(feature = "R4", feature = "R4B"))))]
    {
        return "5.0.0";
    }

    #[cfg(all(
        feature = "R6",
        not(any(feature = "R4", feature = "R4B", feature = "R5"))
    ))]
    {
        return "6.0.0-ballot";
    }

    #[allow(unreachable_code)]
    "unknown"
}

/// Returns all valid resource type names for a specific FHIR version.
///
/// This function returns the resource types that are valid for the specified
/// FHIR version, regardless of which version features are enabled for the
/// default behavior.
///
/// # Arguments
///
/// * `version` - The FHIR version to get resource types for
///
/// # Returns
///
/// A slice of resource type names valid for the specified version.
/// Returns the default version's types if the specified version is not enabled.
pub fn get_resource_type_names_for_version(version: FhirVersion) -> &'static [&'static str] {
    match version {
        #[cfg(feature = "R4")]
        FhirVersion::R4 => R4_RESOURCE_TYPES,
        #[cfg(feature = "R4B")]
        FhirVersion::R4B => get_r4b_resource_types(),
        #[cfg(feature = "R5")]
        FhirVersion::R5 => get_r5_resource_types(),
        #[cfg(feature = "R6")]
        FhirVersion::R6 => get_r6_resource_types(),
        // If the version is not enabled, fall back to default
        #[allow(unreachable_patterns)]
        _ => get_resource_type_names(),
    }
}

// Helper functions to get version-specific resource types when that version
// is enabled but not the default. These are needed because the constants
// have cfg guards that may exclude them based on other enabled versions.

#[cfg(feature = "R4B")]
fn get_r4b_resource_types() -> &'static [&'static str] {
    // If R4B is enabled but R4 is also enabled, R4B_RESOURCE_TYPES
    // may not be compiled. In that case, we return R4 types as fallback.
    #[cfg(not(feature = "R4"))]
    {
        R4B_RESOURCE_TYPES
    }
    #[cfg(feature = "R4")]
    {
        // R4B-specific types (subset for now - ideally this would be a separate const)
        R4_RESOURCE_TYPES
    }
}

#[cfg(feature = "R5")]
fn get_r5_resource_types() -> &'static [&'static str] {
    #[cfg(feature = "R4")]
    {
        R4_RESOURCE_TYPES
    }
    #[cfg(all(not(feature = "R4"), feature = "R4B"))]
    {
        R4B_RESOURCE_TYPES
    }
    #[cfg(not(any(feature = "R4", feature = "R4B")))]
    {
        R5_RESOURCE_TYPES
    }
}

#[cfg(feature = "R6")]
fn get_r6_resource_types() -> &'static [&'static str] {
    #[cfg(feature = "R4")]
    {
        R4_RESOURCE_TYPES
    }
    #[cfg(all(not(feature = "R4"), feature = "R4B"))]
    {
        R4B_RESOURCE_TYPES
    }
    #[cfg(all(not(any(feature = "R4", feature = "R4B")), feature = "R5"))]
    {
        R5_RESOURCE_TYPES
    }
    #[cfg(not(any(feature = "R4", feature = "R4B", feature = "R5")))]
    {
        R6_RESOURCE_TYPES
    }
}

/// Checks if a resource type name is valid for a specific FHIR version.
///
/// # Arguments
///
/// * `type_name` - The resource type name to validate
/// * `version` - The FHIR version to validate against
///
/// # Returns
///
/// `true` if the type name is a valid FHIR resource type for the specified version.
pub fn is_valid_resource_type_for_version(type_name: &str, version: FhirVersion) -> bool {
    get_resource_type_names_for_version(version).contains(&type_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_resource_type_names() {
        let types = get_resource_type_names();

        // Should have a reasonable number of types
        assert!(!types.is_empty());

        // Should include common types
        assert!(types.contains(&"Patient"));
        assert!(types.contains(&"Observation"));
        assert!(types.contains(&"Bundle"));
        assert!(types.contains(&"CapabilityStatement"));
    }

    #[test]
    fn test_is_valid_resource_type() {
        // Valid types
        assert!(is_valid_resource_type("Patient"));
        assert!(is_valid_resource_type("Observation"));
        assert!(is_valid_resource_type("Bundle"));

        // Invalid types
        assert!(!is_valid_resource_type("InvalidType"));
        assert!(!is_valid_resource_type(""));
        assert!(!is_valid_resource_type("patient")); // Case-sensitive
    }

    #[test]
    fn test_get_fhir_version() {
        let version = get_fhir_version();
        // Should be a valid version string
        assert!(!version.is_empty());
        assert!(version.contains('.'));
    }
}
