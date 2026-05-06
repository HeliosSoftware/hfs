mod tests {

    use crate::common::fixtures::{load_profile, local_terminology_r5};
    use fhir_validation::profile::extract::extract_r5_structure_definition_profile;
    use fhir_validation::profile::profile_registry::ProfileRegistry;
    use fhir_validation::profile::types::{
        ExtractedElementRule, ExtractedProfile, ExtractedTypeConstraint,
    };
    use fhir_validation::profile::validate::validate_profile;
    use fhir_validation::validation_context::{ValidationContext, ValidationState};
    use fhir_validation::{
        R5FhirPathEvaluator, Severity, StructureDefinitionKind, TypeDerivationRule,
        TypeProfileMatchMode, Validator,
    };
    use helios_fhir::FhirVersion;
    use helios_fhir::r5::StructureDefinition;
    use helios_fhir::r5::{Bundle, Parameters, Patient, Resource};
    use serde_json::json;

    fn validator() -> Validator {
        Validator::default()
    }
    fn load_atrius_profile() -> ExtractedProfile {
        load_profile(FhirVersion::R5, "profile/atrius-profile.json")
    }

    fn empty_profile() -> ExtractedProfile {
        let mut profile = load_atrius_profile();
        profile.invariants.clear();
        profile.element_rules.clear();
        profile
    }
    fn run_validate_profile<T: serde::Serialize>(
        validator: &Validator,
        resource: &T,
        resource_type: &str,
        profile: &ExtractedProfile,
        evaluator: &dyn fhir_validation::evaluators::FhirPathEvaluator,
        registry: Option<&ProfileRegistry>,
    ) -> Vec<fhir_validation::ValidationIssue> {
        let extracted_profile_map = std::collections::HashMap::new();
        let term = local_terminology_r5();
        let ctx = ValidationContext {
            fhir_version: FhirVersion::R5,
            validator,
            terminology: Some(&term),
            evaluator,
            runtime_profile_registry: registry,
            extracted_profile_map: &extracted_profile_map,
        };
        let mut state = ValidationState::default();
        validate_profile(&ctx, &mut state, resource, resource_type, profile)
    }
    #[test]
    fn fixed_constraint_fails_when_value_differs() {
        let mut profile = load_atrius_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.active".to_string(),
            path: "Patient.active".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: Some(
                fhir_validation::profile::types::ExtractedValueConstraint::Fixed(json!(true)),
            ),
            type_constraints: vec![],
            slicing: None,
            slice_name: None,
            ..Default::default()
        });

        let patient_json = r#"
    {
      "resourceType": "Patient",
      "active": false
    }
    "#;

        let patient: Patient = serde_json::from_str(patient_json)
            .expect("Patient JSON should deserialize into R5 Patient");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient.clone())));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &profile,
            &evaluator,
            None,
        );

        assert!(issues.iter().any(|i| i.fhir_path == "Patient.active"));
        assert!(issues.iter().any(|i| i.code == "value"));
        assert!(issues.iter().any(|i| {
            i.diagnostics
                .contains("Element 'Patient.active' does not satisfy fixed constraint")
        }));
    }

    #[test]
    fn fixed_constraint_passes_when_value_matches() {
        let mut profile = load_atrius_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.active".to_string(),
            path: "Patient.active".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: Some(
                fhir_validation::profile::types::ExtractedValueConstraint::Fixed(json!(true)),
            ),
            type_constraints: vec![],
            slicing: None,
            slice_name: None,
            ..Default::default()
        });

        let patient_json = r#"
    {
      "resourceType": "Patient",
      "active": true,
      "name": [
        {
          "family": "Doe"
        }
      ]
    }
    "#;

        let patient: Patient = serde_json::from_str(patient_json)
            .expect("Patient JSON should deserialize into R5 Patient");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient.clone())));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            !issues
                .iter()
                .any(|i| i.fhir_path == "Patient.active" && i.code == "value")
        );
    }

    #[test]
    fn pattern_constraint_matches_subset_object() {
        let mut profile = load_atrius_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.identifier".to_string(),
            path: "Patient.identifier".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: Some(
                fhir_validation::profile::types::ExtractedValueConstraint::Pattern(json!([
                    {
                        "system": "http://atrius.health/mrn"
                    }
                ])),
            ),
            type_constraints: vec![],
            slicing: None,
            slice_name: None,
            ..Default::default()
        });

        let patient_json = r#"
    {
      "resourceType": "Patient",
      "identifier": [
        {
          "system": "http://atrius.health/mrn",
          "value": "12345"
        }
      ]
    }
    "#;

        let patient: Patient = serde_json::from_str(patient_json)
            .expect("Patient JSON should deserialize into R5 Patient");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient.clone())));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            !issues
                .iter()
                .any(|i| i.fhir_path == "Patient.identifier" && i.code == "value")
        );
    }

    #[test]
    fn pattern_constraint_fails_when_subset_does_not_match() {
        let mut profile = load_atrius_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.identifier".to_string(),
            path: "Patient.identifier".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: Some(
                fhir_validation::profile::types::ExtractedValueConstraint::Pattern(json!([
                    {
                        "system": "http://atrius.health/mrn"
                    }
                ])),
            ),
            type_constraints: vec![],
            slicing: None,
            slice_name: None,
            ..Default::default()
        });

        let patient_json = r#"
    {
      "resourceType": "Patient",
      "identifier": [
        {
          "system": "http://example.org/other",
          "value": "12345"
        }
      ]
    }
    "#;

        let patient: Patient = serde_json::from_str(patient_json)
            .expect("Patient JSON should deserialize into R5 Patient");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient.clone())));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &profile,
            &evaluator,
            None,
        );
        // println!("{:?}", issues);
        assert!(issues.iter().any(|i| i.fhir_path == "Patient.identifier"));
        assert!(issues.iter().any(|i| i.code == "value"));
        assert!(issues.iter().any(|i| {
            i.diagnostics
                .contains("Element 'Patient.identifier' does not satisfy pattern constraint")
        }));
    }
    #[test]
    fn type_code_constraint_allows_matching_choice_type() {
        let mut profile = load_atrius_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.deceased[x]".to_string(),
            path: "Patient.deceased[x]".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: None,
            type_constraints: vec![ExtractedTypeConstraint {
                code: "boolean".to_string(),
                profiles: Vec::new(),
                target_profiles: Vec::new(),
                ..Default::default()
            }],
            slicing: None,
            slice_name: None,
            ..Default::default()
        });

        let patient_json = r#"
        {
          "resourceType": "Patient",
          "deceasedBoolean": true
        }
        "#;

        let patient: Patient = serde_json::from_str(patient_json)
            .expect("Patient JSON should deserialize into R5 Patient");

        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient.clone())));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            !issues
                .iter()
                .any(|i| { i.fhir_path == "Patient.deceased[x]" && i.code == "structure" })
        );
    }

    #[test]
    fn type_code_constraint_rejects_non_matching_choice_type() {
        let mut profile = empty_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.deceased[x]".to_string(),
            path: "Patient.deceased[x]".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: None,
            type_constraints: vec![ExtractedTypeConstraint {
                code: "boolean".to_string(),
                profiles: Vec::new(),
                target_profiles: Vec::new(),
                ..Default::default()
            }],
            slicing: None,
            slice_name: None,
            ..Default::default()
        });

        let patient_json = json!({
            "resourceType": "Patient",
            "deceasedDateTime": "2020-01-01T00:00:00Z"
        });

        let evaluator_patient: Patient = serde_json::from_str(r#"{ "resourceType": "Patient" }"#)
            .expect("Minimal Patient JSON should deserialize into R5 Patient");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(evaluator_patient)));

        let issues = run_validate_profile(
            &validator(),
            &patient_json,
            "Patient",
            &profile,
            &evaluator,
            None,
        );

        assert!(issues.iter().any(|i| i.fhir_path == "Patient.deceased[x]"));
        assert!(issues.iter().any(|i| i.code == "structure"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Patient.deceased[x]' uses disallowed type(s) 'DateTime'. Allowed types: boolean."
            )
        }));
    }

    #[test]
    fn type_code_constraint_rejects_multiple_choice_representations_in_same_object() {
        let mut profile = empty_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.deceased[x]".to_string(),
            path: "Patient.deceased[x]".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: None,
            type_constraints: vec![ExtractedTypeConstraint {
                code: "boolean".to_string(),
                profiles: Vec::new(),
                target_profiles: Vec::new(),
                ..Default::default()
            }],
            slicing: None,
            slice_name: None,
            ..Default::default()
        });

        let patient_json = json!({
            "resourceType": "Patient",
            "deceasedBoolean": true,
            "deceasedDateTime": "2020-01-01T00:00:00Z"
        });

        let evaluator_patient: Patient = serde_json::from_str(r#"{ "resourceType": "Patient" }"#)
            .expect("Minimal Patient JSON should deserialize into R5 Patient");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(evaluator_patient)));

        let issues = run_validate_profile(
            &validator(),
            &patient_json,
            "Patient",
            &profile,
            &evaluator,
            None,
        );

        assert!(issues.iter().any(|i| i.fhir_path == "Patient.deceased[x]"));
        assert!(issues.iter().any(|i| i.code == "structure"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Patient.deceased[x]' has multiple [x] representations present in the same object"
            )
        }));
    }

    #[test]
    fn type_code_constraint_checks_choice_types_inside_array_of_backbone_elements() {
        let mut profile = empty_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Observation.component.value[x]".to_string(),
            path: "Observation.component.value[x]".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: None,
            type_constraints: vec![ExtractedTypeConstraint {
                code: "Quantity".to_string(),
                profiles: Vec::new(),
                target_profiles: Vec::new(),
                ..Default::default()
            }],
            slicing: None,
            slice_name: None,
            ..Default::default()
        });

        let observation_json = json!({
            "resourceType": "Observation",
            "status": "final",
            "code": {
                "text": "Blood pressure panel"
            },
            "component": [
                {
                    "code": { "text": "Systolic" },
                    "valueQuantity": { "value": 120, "unit": "mmHg" }
                },
                {
                    "code": { "text": "Interpretation" },
                    "valueString": "normal"
                }
            ]
        });

        let evaluator_observation: helios_fhir::r5::Observation = serde_json::from_str(
            r#"{
              "resourceType": "Observation",
              "status": "final",
              "code": { "text": "placeholder" }
            }"#,
        )
        .expect("Minimal Observation JSON should deserialize into R5 Observation");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Observation(Box::new(evaluator_observation)));

        let issues = run_validate_profile(
            &validator(),
            &observation_json,
            "Observation",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Observation.component.value[x]")
        );
        assert!(issues.iter().any(|i| i.code == "structure"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Observation.component.value[x]' uses disallowed type(s) 'String'. Allowed types: Quantity."
            )
        }));
    }

    #[test]
    fn target_profile_constraint_allows_matching_reference_target_type() {
        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/observation-subject-patient".to_string(),
            version: None,
            name: Some("ObservationSubjectPatient".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.subject".to_string(),
                path: "Observation.subject".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Reference".to_string(),
                    profiles: Vec::new(),
                    target_profiles: vec![
                        "http://hl7.org/fhir/StructureDefinition/Patient".to_string(),
                    ],
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let observation_json = r#"
        {
          "resourceType": "Observation",
          "status": "final",
          "code": { "text": "Example" },
          "subject": { "reference": "Patient/123" }
        }
        "#;

        let observation: helios_fhir::r5::Observation = serde_json::from_str(observation_json)
            .expect("Observation JSON should deserialize into R5 Observation");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Observation(Box::new(observation.clone())));

        let issues = run_validate_profile(
            &validator(),
            &observation,
            "Observation",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            !issues
                .iter()
                .any(|i| { i.fhir_path == "Observation.subject" && i.code == "structure" })
        );
    }

    #[test]
    fn target_profile_constraint_rejects_non_matching_reference_target_type() {
        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/observation-subject-patient".to_string(),
            version: None,
            name: Some("ObservationSubjectPatient".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.subject".to_string(),
                path: "Observation.subject".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Reference".to_string(),
                    profiles: Vec::new(),
                    target_profiles: vec![
                        "http://hl7.org/fhir/StructureDefinition/Patient".to_string(),
                    ],
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let observation_json = r#"
        {
          "resourceType": "Observation",
          "status": "final",
          "code": { "text": "Example" },
          "subject": { "reference": "Practitioner/123" }
        }
        "#;

        let observation: helios_fhir::r5::Observation = serde_json::from_str(observation_json)
            .expect("Observation JSON should deserialize into R5 Observation");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Observation(Box::new(observation.clone())));

        let issues = run_validate_profile(
            &validator(),
            &observation,
            "Observation",
            &profile,
            &evaluator,
            None,
        );

        assert!(issues.iter().any(|i| i.fhir_path == "Observation.subject"));
        assert!(issues.iter().any(|i| i.code == "structure"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Observation.subject' references resource type 'Practitioner', which is not allowed by the profile. Allowed target types: Patient."
            )
        }));
    }

    #[test]
    fn target_profile_constraint_checks_references_inside_array_paths() {
        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-careplan".to_string(),
            version: None,
            name: Some("ContainerCarePlan".to_string()),
            title: None,
            resource_type: "CarePlan".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "CarePlan.activity.reference".to_string(),
                path: "CarePlan.activity.reference".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Reference".to_string(),
                    profiles: Vec::new(),
                    target_profiles: vec![
                        "http://hl7.org/fhir/StructureDefinition/ServiceRequest".to_string(),
                    ],
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let care_plan = json!({
            "resourceType": "CarePlan",
            "status": "active",
            "intent": "plan",
            "activity": [
                {
                    "reference": {
                        "reference": "ServiceRequest/123"
                    }
                },
                {
                    "reference": {
                        "reference": "Observation/456"
                    }
                }
            ]
        });

        let evaluator_care_plan: helios_fhir::r5::CarePlan = serde_json::from_str(
            r#"{
          "resourceType": "CarePlan",
          "status": "active",
          "intent": "plan"
        }"#,
        )
        .expect("Minimal CarePlan JSON should deserialize into R5 CarePlan");

        let evaluator = R5FhirPathEvaluator::new(Resource::CarePlan(Box::new(evaluator_care_plan)));

        let issues = run_validate_profile(
            &validator(),
            &care_plan,
            "CarePlan",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "CarePlan.activity.reference")
        );
        assert!(issues.iter().any(|i| i.code == "structure"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'CarePlan.activity.reference' references resource type 'Observation', which is not allowed by the profile. Allowed target types: ServiceRequest."
            )
        }));
    }

    #[test]
    fn target_profile_constraint_allows_matching_contained_local_reference_when_resource_is_inline()
    {
        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/observation-subject-patient".to_string(),
            version: None,
            name: Some("ObservationSubjectPatient".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.subject".to_string(),
                path: "Observation.subject".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Reference".to_string(),
                    profiles: Vec::new(),
                    target_profiles: vec![
                        "http://hl7.org/fhir/StructureDefinition/Patient".to_string(),
                    ],
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let observation_json = r##"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Example" },
      "contained": [
        {
          "resourceType": "Patient",
          "id": "p1"
        }
      ],
      "subject": { "reference": "#p1" }
    }"##;

        let observation: helios_fhir::r5::Observation = serde_json::from_str(observation_json)
            .expect("Observation JSON should deserialize into R5 Observation");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Observation(Box::new(observation.clone())));

        let issues = run_validate_profile(
            &validator(),
            &observation,
            "Observation",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            !issues
                .iter()
                .any(|i| i.fhir_path == "Observation.subject" && i.code == "structure")
        );
    }

    #[test]
    fn target_profile_constraint_rejects_non_matching_contained_local_reference_when_resource_is_inline()
     {
        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/observation-subject-patient".to_string(),
            version: None,
            name: Some("ObservationSubjectPatient".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.subject".to_string(),
                path: "Observation.subject".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Reference".to_string(),
                    profiles: Vec::new(),
                    target_profiles: vec![
                        "http://hl7.org/fhir/StructureDefinition/Patient".to_string(),
                    ],
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let observation_json = r##"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Example" },
      "contained": [
        {
          "resourceType": "Practitioner",
          "id": "p1"
        }
      ],
      "subject": { "reference": "#p1" }
    }"##;

        let observation: helios_fhir::r5::Observation = serde_json::from_str(observation_json)
            .expect("Observation JSON should deserialize into R5 Observation");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Observation(Box::new(observation.clone())));

        let issues = run_validate_profile(
            &validator(),
            &observation,
            "Observation",
            &profile,
            &evaluator,
            None,
        );

        assert!(issues.iter().any(|i| i.fhir_path == "Observation.subject"));
        assert!(issues.iter().any(|i| i.code == "structure"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Observation.subject' references resource type 'Practitioner', which is not allowed by the profile. Allowed target types: Patient."
            )
        }));
    }
    #[test]
    fn type_profile_constraint_falls_back_to_matching_resource_type_when_no_declared_profile_is_present()
     {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                .to_string(),
            version: None,
            name: Some("AtriusRelatedObservation".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: Vec::new(),
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation"
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));

        let issues = run_validate_profile(
            &validator(),
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(!issues.iter().any(|i| i.fhir_path == "Parameters.parameter[0].resource" && i.code == "structure"));
        assert!(issues.iter().any(
            |i| i.fhir_path == "Parameters.parameter[0].resource" && i.code == "business-rule"
        ));
    }

    #[test]
    fn type_profile_constraint_recursively_validates_nested_resource_against_required_profile() {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                .to_string(),
            version: None,
            name: Some("AtriusRelatedObservation".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.status".to_string(),
                path: "Observation.status".to_string(),
                min: Some(1),
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: Vec::new(),
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            ]
                        }
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));

        let issues = run_validate_profile(
            &validator(),
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );
        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource.status")
        );
        assert!(
            issues
                .iter()
                .any(|i| i.instance_path.as_deref()
                    == Some("Parameters.parameter[0].resource.status"))
        );
        assert!(issues.iter().any(|i| i.code == "required"));
    }

    #[test]
    fn type_profile_constraint_prefixes_root_level_nested_cycle_issue_to_parent_path() {
        let profile = ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                .to_string(),
            version: None,
            name: Some("AtriusRelatedObservation".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.hasMember".to_string(),
                path: "Observation.hasMember".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let mut registry = ProfileRegistry::new();
        registry.insert(profile.clone());

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            ]
                        },
                        "hasMember": {
                            "resourceType": "Observation",
                            "meta": {
                                "profile": [
                                    "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                                ]
                            }
                        }
                    }
                }
            ]
        });

        let container_profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));

        let issues = run_validate_profile(
            &validator(),
            &container,
            "Parameters",
            &container_profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource.hasMember")
        );
        assert!(
            issues.iter().any(|i| i.instance_path.as_deref()
                == Some("Parameters.parameter[0].resource.hasMember"))
        );
        assert!(issues.iter().any(|i| i.code == "business-rule"));
        assert!(
            issues
                .iter()
                .any(|i| { i.diagnostics.contains("validation cycle was detected") })
        );
    }

    #[test]
    fn type_profile_constraint_prefixes_deeper_nested_issue_to_full_parent_path() {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                .to_string(),
            version: None,
            name: Some("AtriusRelatedObservation".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.code.text".to_string(),
                path: "Observation.code.text".to_string(),
                min: Some(1),
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: Vec::new(),
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            ]
                        },
                        "code": {}
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));

        let issues = run_validate_profile(
            &validator(),
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource.code.text")
        );
        assert!(
            issues.iter().any(|i| i.instance_path.as_deref()
                == Some("Parameters.parameter[0].resource.code.text"))
        );
        assert!(issues.iter().any(|i| i.code == "required"));
    }

    #[test]
    fn type_profile_fallback_disabled_produces_error_instead_of_warning() {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                .to_string(),
            version: None,
            name: Some("AtriusRelatedObservation".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: Vec::new(),
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation"
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));
        let mut validator = validator();
        validator.config.allow_type_profile_resource_type_fallback = false;

        let issues = run_validate_profile(
            &validator,
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource")
        );
        assert!(issues.iter().any(|i| i.code == "structure"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Parameters.parameter.resource' does not explicitly declare any of the required profiles, and resourceType fallback is disabled."
            )
        }));
    }

    #[test]
    fn type_profile_fallback_warning_can_be_disabled() {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                .to_string(),
            version: None,
            name: Some("AtriusRelatedObservation".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: Vec::new(),
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation"
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));
        let mut validator = validator();
        validator.config.warn_on_type_profile_fallback = false;

        let issues = run_validate_profile(
            &validator,
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(!issues.iter().any(|i| {
            i.fhir_path == "Parameters.parameter[0].resource" && i.code == "business-rule"
        }));
        assert!(
            !issues
                .iter()
                .any(|i| { i.diagnostics.contains("Falling back to resourceType match") })
        );
    }

    #[test]
    fn type_profile_fallback_recursion_can_be_disabled() {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                .to_string(),
            version: None,
            name: Some("AtriusRelatedObservation".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.status".to_string(),
                path: "Observation.status".to_string(),
                min: Some(1),
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: Vec::new(),
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation"
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));
        let mut validator = validator();
        validator.config.recurse_on_type_profile_fallback = false;

        let issues = run_validate_profile(
            &validator,
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            !issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource.status")
        );
        assert!(!issues.iter().any(|i| i.code == "required"));
    }

    #[test]
    fn profile_cycle_and_depth_warnings_can_be_silently_skipped() {
        let profile = ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                .to_string(),
            version: None,
            name: Some("AtriusRelatedObservation".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Observation.hasMember".to_string(),
                path: "Observation.hasMember".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let mut registry = ProfileRegistry::new();
        registry.insert(profile.clone());

        let _container_profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                            ]
                        },
                        "hasMember": {
                            "resourceType": "Observation",
                            "meta": {
                                "profile": [
                                    "http://atrius.health/fhir/StructureDefinition/atrius-related-observation"
                                ]
                            }
                        }
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));
        let mut validator = validator();
        validator.config.warn_on_profile_cycle = false;
        validator.config.warn_on_profile_recursion_depth_reached = false;

        let issues = run_validate_profile(
            &validator,
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(!issues.iter().any(|i| i.code == "business-rule"
            && i.diagnostics.contains("validation cycle was detected")));
        assert!(!issues.iter().any(
            |i| i.code == "business-rule" && i.diagnostics.contains("maximum recursion depth")
        ));
    }

    #[test]
    fn type_profile_unknown_profile_produces_error_by_default() {
        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/unknown-observation-profile"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let registry = ProfileRegistry::new();

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/unknown-observation-profile"
                            ]
                        }
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));

        let issues = run_validate_profile(
            &validator(),
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource")
        );
        assert!(issues.iter().any(|i| i.code == "not-found"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Parameters.parameter.resource' requires unknown profile(s): http://atrius.health/fhir/StructureDefinition/unknown-observation-profile."
            )
        }));
    }

    #[test]
    fn type_profile_unknown_profile_can_warn_instead_of_error() {
        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/unknown-observation-profile"
                            .to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let registry = ProfileRegistry::new();

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/unknown-observation-profile"
                            ]
                        }
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));
        let mut validator = validator();
        validator.config.error_on_unknown_profile = false;
        validator.config.warn_on_unknown_profile = true;

        let issues = run_validate_profile(
            &validator,
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource")
        );
        assert!(issues.iter().any(|i| i.code == "not-found"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Parameters.parameter.resource' references unknown profile(s): http://atrius.health/fhir/StructureDefinition/unknown-observation-profile."
            )
        }));
        assert!(
            !issues
                .iter()
                .any(|i| { i.diagnostics.contains("requires unknown profile(s)") })
        );
    }

    #[test]
    fn type_profile_match_mode_any_accepts_any_single_matching_declared_profile() {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/profile-a".to_string(),
            version: None,
            name: Some("ProfileA".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: Vec::new(),
        });
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/profile-b".to_string(),
            version: None,
            name: Some("ProfileB".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: Vec::new(),
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/profile-a".to_string(),
                        "http://atrius.health/fhir/StructureDefinition/profile-b".to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/profile-a"
                            ]
                        }
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));
        let mut validator = validator();
        validator.config.type_profile_match_mode = TypeProfileMatchMode::Any;

        let issues = run_validate_profile(
            &validator,
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(!issues.iter().any(|i| i.fhir_path == "Parameters.parameter[0].resource" && i.code == "structure"));
    }

    #[test]
    fn type_profile_match_mode_all_requires_all_profiles_and_fails_on_partial_match() {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/profile-a".to_string(),
            version: None,
            name: Some("ProfileA".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: Vec::new(),
        });
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/profile-b".to_string(),
            version: None,
            name: Some("ProfileB".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: Vec::new(),
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/profile-a".to_string(),
                        "http://atrius.health/fhir/StructureDefinition/profile-b".to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/profile-a"
                            ]
                        }
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
              "resourceType": "Parameters",
              "parameter": [
                {
                  "name": "payload"
                }
              ]
            }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));
        let mut validator = validator();
        validator.config.type_profile_match_mode = TypeProfileMatchMode::All;

        let issues = run_validate_profile(
            &validator,
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource")
        );
        assert!(issues.iter().any(|i| i.code == "structure"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains("Element 'Parameters.parameter.resource' does not declare the required profile match.")
        }));
        assert!(
            issues
                .iter()
                .any(|i| { i.diagnostics.contains("Match mode: All") })
        );
    }
    #[test]
    fn type_profile_mixed_known_and_unknown_profiles_still_errors_on_unknown_by_default() {
        let mut registry = ProfileRegistry::new();
        registry.insert(ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/profile-known".to_string(),
            version: None,
            name: Some("ProfileKnown".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: Vec::new(),
        });

        let profile = ExtractedProfile {
            url: "http://example.org/StructureDefinition/container-parameters".to_string(),
            version: None,
            name: Some("ContainerParameters".to_string()),
            title: None,
            resource_type: "Parameters".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::Resource,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![ExtractedElementRule {
                id: "Parameters.parameter.resource".to_string(),
                path: "Parameters.parameter.resource".to_string(),
                min: None,
                max: None,
                binding: None,
                constraints: Vec::new(),
                value_constraint: None,
                type_constraints: vec![ExtractedTypeConstraint {
                    code: "Observation".to_string(),
                    profiles: vec![
                        "http://atrius.health/fhir/StructureDefinition/profile-known".to_string(),
                        "http://atrius.health/fhir/StructureDefinition/profile-unknown".to_string(),
                    ],
                    target_profiles: Vec::new(),
                    ..Default::default()
                }],
                slicing: None,
                slice_name: None,
                ..Default::default()
            }],
        };

        let container = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "payload",
                    "resource": {
                        "resourceType": "Observation",
                        "meta": {
                            "profile": [
                                "http://atrius.health/fhir/StructureDefinition/profile-known"
                            ]
                        }
                    }
                }
            ]
        });

        let evaluator_parameters: Parameters = serde_json::from_str(
            r#"{
          "resourceType": "Parameters",
          "parameter": [
            {
              "name": "payload"
            }
          ]
        }"#,
        )
        .expect("Minimal Parameters JSON should deserialize into R5 Parameters");

        let evaluator =
            R5FhirPathEvaluator::new(Resource::Parameters(Box::new(evaluator_parameters)));

        let issues = run_validate_profile(
            &validator(),
            &container,
            "Parameters",
            &profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Parameters.parameter[0].resource")
        );
        assert!(issues.iter().any(|i| i.code == "not-found"));
        assert!(issues.iter().any(|i| {
            i.diagnostics.contains(
                "Element 'Parameters.parameter.resource' requires unknown profile(s): http://atrius.health/fhir/StructureDefinition/profile-unknown."
            )
        }));
    }

    /// Same differential content as `framework::extract::tests::TYPE_AGGREGATION_VERSIONING_SD`.
    const PATIENT_LINK_AGGREGATION_SD: &str = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "patient-link-aggregation",
      "url": "http://example.org/fhir/StructureDefinition/patient-link-aggregation",
      "name": "PatientLinkAggregation",
      "title": "Patient link aggregation",
      "status": "draft",
      "date": "2026-01-01",
      "publisher": "Test",
      "kind": "resource",
      "abstract": false,
      "type": "Patient",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Patient.link.other",
            "path": "Patient.link.other",
            "type": [
              {
                "code": "Reference",
                "aggregation": ["referenced"],
                "versioning": "independent"
              }
            ]
          }
        ]
      }
    }
    "#;

    #[test]
    fn extracted_profile_enforces_reference_versioning_from_structure_definition() {
        let sd: StructureDefinition = serde_json::from_str(PATIENT_LINK_AGGREGATION_SD)
            .expect("aggregation SD JSON should deserialize into R5 StructureDefinition");
        let profile = extract_r5_structure_definition_profile(&sd)
            .expect("extracted profile should include aggregation and versioning");

        let patient_json = r#"
        {
          "resourceType": "Patient",
          "link": [
            {
              "other": { "reference": "Patient/1/_history/2" }
            }
          ]
        }
        "#;

        let patient: Patient = serde_json::from_str(patient_json)
            .expect("Patient JSON with versioned reference should deserialize");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient.clone())));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            issues.iter().any(|i| {
                i.fhir_path == "Patient.link.other"
                    && i.code == "structure"
                    && i.diagnostics.contains("aggregation/versioning")
            }),
            "expected aggregation/versioning structure issue, got {issues:?}"
        );
    }

    #[test]
    fn bundled_aggregation_requires_bundle_resource_and_matching_entry() {
        const BUNDLE_SUBJECT_SD: &str = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bundle-subject-bundled",
      "url": "http://example.org/fhir/StructureDefinition/bundle-subject-bundled",
      "name": "BundleSubjectBundled",
      "title": "Bundle subject bundled",
      "status": "draft",
      "date": "2026-01-01",
      "publisher": "Test",
      "kind": "resource",
      "abstract": false,
      "type": "Bundle",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Bundle",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Bundle.entry.resource.subject",
            "path": "Bundle.entry.resource.subject",
            "type": [{ "code": "Reference", "aggregation": ["bundled"] }]
          }
        ]
      }
    }
    "#;

        let sd: StructureDefinition = serde_json::from_str(BUNDLE_SUBJECT_SD)
            .expect("bundle SD JSON should deserialize into R5 StructureDefinition");
        let profile =
            extract_r5_structure_definition_profile(&sd).expect("bundle profile should extract");

        let bundle_pass_json = r#"
        {
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "fullUrl": "urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
              "resource": {
                "resourceType": "Patient",
                "id": "p1",
                "name": [{ "family": "Test" }]
              }
            },
            {
              "resource": {
                "resourceType": "Observation",
                "id": "o1",
                "status": "final",
                "code": { "text": "x" },
                "subject": { "reference": "urn:uuid:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee" }
              }
            }
          ]
        }
        "#;

        let bundle_fail_json = r#"
        {
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "Observation",
                "id": "o1",
                "status": "final",
                "code": { "text": "x" },
                "subject": { "reference": "Patient/999" }
              }
            }
          ]
        }
        "#;

        // Validate against JSON `Value`, not typed `Bundle`, so `fullUrl` stays the
        // FHIR string form expected by aggregation checks (typed round-trips can
        // reshape primitive wrappers).
        let bundle_pass_value: serde_json::Value =
            serde_json::from_str(bundle_pass_json).expect("bundle JSON should parse to Value");
        let bundle_pass_for_eval: Bundle = serde_json::from_value(bundle_pass_value.clone())
            .expect("bundle JSON should deserialize into R5 Bundle");
        let evaluator_pass =
            R5FhirPathEvaluator::new(Resource::Bundle(Box::new(bundle_pass_for_eval)));

        let issues_pass = run_validate_profile(
            &validator(),
            &bundle_pass_value,
            "Bundle",
            &profile,
            &evaluator_pass,
            None,
        );

        assert!(
            !issues_pass.iter().any(|i| {
                i.fhir_path == "Bundle.entry.resource.subject" && i.code == "structure"
            }),
            "expected no structure issue on bundle with matching entry, got {issues_pass:?}"
        );

        let bundle_fail_value: serde_json::Value =
            serde_json::from_str(bundle_fail_json).expect("bundle JSON should parse to Value");
        let bundle_fail_for_eval: Bundle = serde_json::from_value(bundle_fail_value.clone())
            .expect("bundle JSON should deserialize into R5 Bundle");
        let evaluator_fail =
            R5FhirPathEvaluator::new(Resource::Bundle(Box::new(bundle_fail_for_eval)));

        let issues_fail = run_validate_profile(
            &validator(),
            &bundle_fail_value,
            "Bundle",
            &profile,
            &evaluator_fail,
            None,
        );

        assert!(
            issues_fail.iter().any(|i| {
                i.fhir_path == "Bundle.entry.resource.subject" && i.code == "structure"
            }),
            "expected structure issue when subject reference does not match bundle entries, got {issues_fail:?}"
        );
    }

    #[test]
    fn recurses_into_non_core_base_definition_profile_from_registry() {
        let mut base = empty_profile();
        base.url = "http://example.org/fhir/StructureDefinition/base-patient".to_string();
        base.resource_type = "Patient".to_string();
        base.base_definition = Some("http://hl7.org/fhir/StructureDefinition/Patient".to_string());
        base.element_rules.push(ExtractedElementRule {
            id: "Patient.identifier".to_string(),
            path: "Patient.identifier".to_string(),
            min: Some(1),
            ..Default::default()
        });

        let mut derived = empty_profile();
        derived.url = "http://example.org/fhir/StructureDefinition/derived-patient".to_string();
        derived.resource_type = "Patient".to_string();
        derived.base_definition = Some(base.url.clone());

        let mut registry = ProfileRegistry::default();
        registry.insert(base);

        let patient_json = r#"{ "resourceType": "Patient" }"#;
        let patient: Patient =
            serde_json::from_str(patient_json).expect("Patient JSON should deserialize");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient.clone())));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &derived,
            &evaluator,
            Some(&registry),
        );

        assert!(
            issues
                .iter()
                .any(|i| i.fhir_path == "Patient.identifier" && i.code == "required"),
            "expected required issue inherited from non-core base profile, got {issues:?}"
        );
    }

    #[test]
    fn does_not_recurse_into_core_hl7_base_definition_profile() {
        let mut fake_core = empty_profile();
        fake_core.url = "http://hl7.org/fhir/StructureDefinition/Patient".to_string();
        fake_core.resource_type = "Patient".to_string();
        fake_core.element_rules.push(ExtractedElementRule {
            id: "Patient.identifier".to_string(),
            path: "Patient.identifier".to_string(),
            min: Some(1),
            ..Default::default()
        });

        let mut derived = empty_profile();
        derived.url = "http://example.org/fhir/StructureDefinition/derived-patient".to_string();
        derived.resource_type = "Patient".to_string();
        derived.base_definition =
            Some("http://hl7.org/fhir/StructureDefinition/Patient".to_string());

        let mut registry = ProfileRegistry::default();
        registry.insert(fake_core);

        let patient_json = r#"{ "resourceType": "Patient" }"#;
        let patient: Patient =
            serde_json::from_str(patient_json).expect("Patient JSON should deserialize");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient.clone())));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &derived,
            &evaluator,
            Some(&registry),
        );

        assert!(
            !issues
                .iter()
                .any(|i| i.fhir_path == "Patient.identifier" && i.code == "required"),
            "did not expect recursion into core HL7 base profile, got {issues:?}"
        );
    }

    #[test]
    fn must_support_unpopulated_emits_warning() {
        let mut profile = empty_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.active".to_string(),
            path: "Patient.active".to_string(),
            must_support: Some(true),
            ..Default::default()
        });

        let patient = json!({ "resourceType": "Patient" });
        let patient_typed: Patient =
            serde_json::from_value(patient.clone()).expect("Patient JSON should deserialize");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient_typed)));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &profile,
            &evaluator,
            None,
        );

        assert!(
            issues.iter().any(|i| {
                i.fhir_path == "Patient.active"
                    && i.severity == Severity::Warning
                    && i.code == "structure"
            }),
            "expected mustSupport warning on Patient.active, got {issues:?}"
        );
    }

    #[test]
    fn must_support_unpopulated_emits_error_when_configured() {
        let mut profile = empty_profile();
        profile.element_rules.push(ExtractedElementRule {
            id: "Patient.active".to_string(),
            path: "Patient.active".to_string(),
            must_support: Some(true),
            ..Default::default()
        });

        let patient = json!({ "resourceType": "Patient" });
        let patient_typed: Patient =
            serde_json::from_value(patient.clone()).expect("Patient JSON should deserialize");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient_typed)));

        let mut v = validator();
        v.config.must_support_missing_severity = Severity::Error;

        let issues = run_validate_profile(&v, &patient, "Patient", &profile, &evaluator, None);

        assert!(
            issues.iter().any(|i| {
                i.fhir_path == "Patient.active"
                    && i.severity == Severity::Error
                    && i.code == "structure"
            }),
            "expected mustSupport error on Patient.active, got {issues:?}"
        );
    }

    #[test]
    fn type_profile_extension_declares_profile_via_url() {
        let ext_url = "http://example.org/fhir/StructureDefinition/test-my-extension";
        let ext_profile = ExtractedProfile {
            url: ext_url.to_string(),
            version: None,
            name: None,
            title: None,
            resource_type: "Extension".to_string(),
            base_definition: None,
            snapshot_base_version: None,
            kind: StructureDefinitionKind::ComplexType,
            derivation: TypeDerivationRule::Constraint,
            invariants: Vec::new(),
            element_rules: vec![],
        };

        let mut patient_profile = empty_profile();
        patient_profile.url =
            "http://example.org/fhir/StructureDefinition/test-patient".to_string();
        patient_profile.element_rules = vec![ExtractedElementRule {
            id: "Patient.extension".to_string(),
            path: "Patient.extension".to_string(),
            min: None,
            max: None,
            binding: None,
            constraints: Vec::new(),
            value_constraint: None,
            type_constraints: vec![ExtractedTypeConstraint {
                code: "Extension".to_string(),
                profiles: vec![ext_url.to_string()],
                target_profiles: Vec::new(),
                ..Default::default()
            }],
            slicing: None,
            slice_name: None,
            ..Default::default()
        }];

        let mut registry = ProfileRegistry::new();
        registry.insert(ext_profile);

        let patient = json!({
            "resourceType": "Patient",
            "extension": [
                {
                    "url": ext_url,
                    "valueString": "x"
                }
            ]
        });

        let patient_typed: Patient =
            serde_json::from_value(patient.clone()).expect("Patient JSON should deserialize");
        let evaluator = R5FhirPathEvaluator::new(Resource::Patient(Box::new(patient_typed)));

        let issues = run_validate_profile(
            &validator(),
            &patient,
            "Patient",
            &patient_profile,
            &evaluator,
            Some(&registry),
        );

        assert!(
            !issues.iter().any(|i| i.summary.as_deref()
                == Some("Declared profiles do not satisfy type.profile requirement")),
            "extension url should satisfy type.profile like meta.profile, got {issues:?}"
        );
    }
}
