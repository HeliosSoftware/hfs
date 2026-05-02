//! Profile validation against [`StructureDefinition-AtriusPatient.json`](../../fixtures/r4/profiles/StructureDefinition-AtriusPatient.json).

mod tests {
    use crate::common::fixtures::{load_profile, local_terminology_r4};
    use fhir_validation::profile::profile_registry::ProfileRegistry;
    use fhir_validation::{R4FhirPathEvaluator, Severity, Validator};
    use helios_fhir::FhirResource;
    use helios_fhir::r4::{Patient, Resource};
    use helios_fhir::FhirVersion;
    use serde_json::json;

    fn r4_evaluator_for(resource: &FhirResource) -> R4FhirPathEvaluator {
        match resource {
            FhirResource::R4(r) => R4FhirPathEvaluator::new((**r).clone()),
            _ => panic!("expected R4 resource"),
        }
    }

    const ATRIUS_PATIENT_PROFILE_URL: &str = "http://atrius.in/StructureDefinition/AtriusPatient";

    fn atrius_patient_registry() -> ProfileRegistry {
        let mut registry = ProfileRegistry::new();
        registry.insert(load_profile(
            FhirVersion::R4,
            "profiles/StructureDefinition-AtriusPatient.json",
        ));
        registry
    }

    fn validator_for_atrius_profile() -> Validator {
        let mut validator = Validator::default();
        // Base is NDHM Patient — not shipped in tests; avoid network and silent skip of base rules.
        validator.config.recurse_on_base_definition = false;
        validator.config.enable_base_definition_url_lookup = false;
        // Identifier.type is extensible to an NDHM ValueSet not bundled here.
        validator.config.strict_extensible_bindings = false;
        validator
    }

    fn patient_resource_with_meta(patient_value: serde_json::Value) -> FhirResource {
        let patient: Patient =
            serde_json::from_value(patient_value).expect("Patient JSON should deserialize");
        FhirResource::R4(Box::new(Resource::Patient(Box::new(patient))))
    }

    #[test]
    fn atrius_patient_fixture_extracts_expected_url_and_type() {
        let profile = load_profile(
            FhirVersion::R4,
            "profiles/StructureDefinition-AtriusPatient.json",
        );
        assert_eq!(profile.url, ATRIUS_PATIENT_PROFILE_URL);
        assert_eq!(profile.resource_type, "Patient");
        assert!(
            profile.base_definition.as_deref().is_some_and(|b| {
                b.contains("nrces.in") && b.ends_with("/StructureDefinition/Patient")
            }),
            "unexpected baseDefinition: {:?}",
            profile.base_definition
        );
        assert!(
            !profile.element_rules.is_empty(),
            "snapshot-first extraction should yield element rules"
        );
    }

    #[test]
    fn atrius_patient_profile_errors_when_identifier_missing() {
        let patient = json!({
            "resourceType": "Patient",
            "meta": { "profile": [ ATRIUS_PATIENT_PROFILE_URL ] },
            "gender": "male",
            "birthDate": "1990-01-01",
            "name": [{ "text": "Example" }]
        });
        let resource = patient_resource_with_meta(patient);
        let registry = atrius_patient_registry();
        let evaluator = r4_evaluator_for(&resource);
        let term = local_terminology_r4();

        let issues = validator_for_atrius_profile().validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &registry,
        );

        let errors: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
            .collect();
        assert!(
            errors.iter().any(|i| {
                i.fhir_path == "Patient.identifier" && i.code == "required"
            }),
            "expected required identifier under AtriusPatient profile, got: {errors:#?}"
        );
    }

    #[test]
    fn atrius_patient_profile_accepts_minimal_conforming_instance() {
        let patient = json!({
            "resourceType": "Patient",
            "meta": { "profile": [ ATRIUS_PATIENT_PROFILE_URL ] },
            "identifier": [{
                "use": "usual",
                "type": {
                    "coding": [{
                        "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                        "code": "MR",
                        "display": "Medical record number"
                    }]
                },
                "system": "http://hospital.example.org/patients",
                "value": "12345"
            }],
            "name": [{ "text": "Example Patient" }],
            "gender": "male",
            "birthDate": "1990-01-01"
        });
        let resource = patient_resource_with_meta(patient);
        let registry = atrius_patient_registry();
        let evaluator = r4_evaluator_for(&resource);
        let term = local_terminology_r4();

        let issues = validator_for_atrius_profile().validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &registry,
        );

        let errors: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
            .collect();
        assert!(
            !errors.iter().any(|i| {
                i.fhir_path == "Patient.identifier" && i.code == "required"
            }),
            "did not expect missing-identifier error for conforming instance: {errors:#?}"
        );
    }
}
