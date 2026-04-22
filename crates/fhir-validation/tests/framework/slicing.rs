use fhir_validation::ValidationConfig;
use fhir_validation::profile::extract::extract_r5_structure_definition_profile;
use fhir_validation::profile::slicing::{validate_slicing, validate_slicing_with_context};
use fhir_validation::profile::types::{
    ExtractedDiscriminatorType, ExtractedProfile, ExtractedSlicingRules,
};
use helios_fhir::FhirVersion;
use helios_fhir::r5::StructureDefinition;

struct StubFhirPathEvaluator;

impl fhir_validation::evaluators::FhirPathEvaluator for StubFhirPathEvaluator {
    fn eval_invariant(
        &self,
        _declared_path: &str,
        _expression: &str,
    ) -> Result<bool, fhir_validation::ValidationError> {
        Ok(true)
    }

    fn eval_invariant_on(
        &self,
        _focus: helios_fhirpath_support::EvaluationResult,
        _declared_path: &str,
        _expression: &str,
    ) -> Result<bool, fhir_validation::ValidationError> {
        Ok(true)
    }

    fn eval_invariants_on(
        &self,
        _focus: helios_fhirpath_support::EvaluationResult,
        invariants: &[fhir_validation::evaluators::InvariantExprRef<'_>],
    ) -> Vec<Result<bool, fhir_validation::ValidationError>> {
        invariants.iter().map(|_| Ok(true)).collect()
    }

    fn eval_path(
        &self,
        _path: &str,
    ) -> Result<Vec<helios_fhirpath_support::EvaluationResult>, fhir_validation::ValidationError>
    {
        Ok(vec![])
    }
}

#[test]
fn extracts_slicing_and_slice_names_from_structure_definition_differential() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile",
      "name": "BloodPressurePanelProfile",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                {
                  "type": "value",
                  "path": "code"
                }
              ],
              "ordered": false,
              "rules": "open"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:diastolic",
            "path": "Observation.component",
            "sliceName": "diastolic",
            "min": 1,
            "max": "1"
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");

    let extracted = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let base_rule = extracted
        .element_rules
        .iter()
        .find(|rule| rule.id == "Observation.component")
        .expect("Base sliced element rule should be extracted");

    let slicing = base_rule
        .slicing
        .as_ref()
        .expect("Base sliced element should carry slicing metadata");

    assert_eq!(slicing.discriminators.len(), 1);
    assert_eq!(
        slicing.discriminators[0].discriminator_type,
        ExtractedDiscriminatorType::Value
    );
    assert_eq!(slicing.discriminators[0].path, "code");
    assert!(!slicing.ordered);
    assert_eq!(slicing.rules, ExtractedSlicingRules::Open);

    let systolic_rule = extracted
        .element_rules
        .iter()
        .find(|rule| rule.id == "Observation.component:systolic")
        .expect("Systolic slice rule should be extracted");
    assert_eq!(systolic_rule.slice_name.as_deref(), Some("systolic"));
    assert_eq!(systolic_rule.min, Some(1));
    assert_eq!(systolic_rule.max.as_deref(), Some("1"));
    assert!(systolic_rule.slicing.is_none());

    let diastolic_rule = extracted
        .element_rules
        .iter()
        .find(|rule| rule.id == "Observation.component:diastolic")
        .expect("Diastolic slice rule should be extracted");
    assert_eq!(diastolic_rule.slice_name.as_deref(), Some("diastolic"));
    assert_eq!(diastolic_rule.min, Some(1));
    assert_eq!(diastolic_rule.max.as_deref(), Some("1"));
    assert!(diastolic_rule.slicing.is_none());
}

#[test]
fn closed_slicing_rejects_unmatched_array_item() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile",
      "name": "BloodPressurePanelProfile",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 0,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8480-6"
                }
              ]
            }
          },
          {
            "id": "Observation.component:diastolic",
            "path": "Observation.component",
            "sliceName": "diastolic",
            "min": 0,
            "max": "1"
          },
          {
            "id": "Observation.component:diastolic.code",
            "path": "Observation.component.code",
            "sliceName": "diastolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8462-4"
                }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8480-6" }
            ]
          }
        },
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "9999-9" }
            ]
          }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component[1]")
    );
    assert!(issues.iter().any(|i| i.code == "structure"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "does not match any declared slice on 'Observation.component', and slicing rules are closed"
        )
    }));
}

#[test]
fn slice_min_cardinality_fails_when_required_slice_is_missing() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile",
      "name": "BloodPressurePanelProfile",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": false,
              "rules": "open"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8480-6"
                }
              ]
            }
          },
          {
            "id": "Observation.component:diastolic",
            "path": "Observation.component",
            "sliceName": "diastolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:diastolic.code",
            "path": "Observation.component.code",
            "sliceName": "diastolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8462-4"
                }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8480-6" }
            ]
          }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component:diastolic")
    );
    assert!(issues.iter().any(|i| i.code == "required"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "Slice 'Observation.component:diastolic' requires at least 1 occurrence(s), but found 0."
        )
    }));
}

#[test]
fn slice_matching_succeeds_for_two_known_components() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile",
      "name": "BloodPressurePanelProfile",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": false,
              "rules": "open"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8480-6"
                }
              ]
            }
          },
          {
            "id": "Observation.component:diastolic",
            "path": "Observation.component",
            "sliceName": "diastolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:diastolic.code",
            "path": "Observation.component.code",
            "sliceName": "diastolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8462-4"
                }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");
    // println!("EXTRACTED RULES = {:#?}", extracted.element_rules);

    let systolic_code_rule = extracted
        .element_rules
        .iter()
        .find(|rule| rule.id == "Observation.component:systolic.code")
        .expect("Systolic discriminator child rule should be extracted");
    assert!(
        systolic_code_rule.value_constraint.is_some(),
        "Expected value_constraint on Observation.component:systolic.code, got: {systolic_code_rule:#?}"
    );

    let diastolic_code_rule = extracted
        .element_rules
        .iter()
        .find(|rule| rule.id == "Observation.component:diastolic.code")
        .expect("Diastolic discriminator child rule should be extracted");
    assert!(
        diastolic_code_rule.value_constraint.is_some(),
        "Expected value_constraint on Observation.component:diastolic.code, got: {diastolic_code_rule:#?}"
    );

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8480-6" }
            ]
          }
        },
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8462-4" }
            ]
          }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");
    // println!("OBSERVATION JSON = {:#?}", observation);

    let issues = validate_slicing(&observation, "Observation", &extracted);
    // println!("SLICING ISSUES = {:#?}", issues);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}
#[test]
fn slice_max_cardinality_fails_when_slice_repeats_too_many_times() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile",
      "name": "BloodPressurePanelProfile",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": false,
              "rules": "open"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 0,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8480-6"
                }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8480-6" }
            ]
          }
        },
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8480-6" }
            ]
          }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component:systolic")
    );
    assert!(issues.iter().any(|i| i.code == "structure"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "Slice 'Observation.component:systolic' allows at most 1 occurrence(s), but found 2.",
        )
    }));
}

#[test]
fn multi_discriminator_requires_all_discriminators_to_match() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile-multi-discriminator",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile-multi-discriminator",
      "name": "BloodPressurePanelProfileMultiDiscriminator",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" },
                { "type": "value", "path": "valueQuantity" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8480-6"
                }
              ]
            }
          },
          {
            "id": "Observation.component:systolic.valueQuantity",
            "path": "Observation.component.valueQuantity",
            "sliceName": "systolic",
            "patternQuantity": {
              "system": "http://unitsofmeasure.org",
              "code": "mm[Hg]"
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8480-6" }
            ]
          },
          "valueQuantity": {
            "system": "http://unitsofmeasure.org",
            "code": "kPa"
          }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component[0]")
    );
    assert!(issues.iter().any(|i| i.code == "structure"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "does not match any declared slice on 'Observation.component', and slicing rules are closed"
        )
    }));
    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component:systolic" && i.code == "required")
    );
}

#[test]
fn slice_conflict_detection_fails_when_one_item_matches_multiple_slices() {
    let sd_json = r#"
        {
          "resourceType": "StructureDefinition",
          "id": "bp-panel-profile-conflict",
          "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile-conflict",
          "name": "BloodPressurePanelProfileConflict",
          "status": "draft",
          "kind": "resource",
          "abstract": false,
          "type": "Observation",
          "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
          "derivation": "constraint",
          "differential": {
            "element": [
              {
                "id": "Observation.component",
                "path": "Observation.component",
                "slicing": {
                  "discriminator": [
                    { "type": "value", "path": "code" }
                  ],
                  "ordered": false,
                  "rules": "closed"
                }
              },
              {
                "id": "Observation.component:slice-a",
                "path": "Observation.component",
                "sliceName": "slice-a",
                "min": 0,
                "max": "1"
              },
              {
                "id": "Observation.component:slice-a.code",
                "path": "Observation.component.code",
                "sliceName": "slice-a",
                "patternCodeableConcept": {
                  "coding": [
                    {
                      "system": "http://loinc.org",
                      "code": "8480-6"
                    }
                  ]
                }
              },
              {
                "id": "Observation.component:slice-b",
                "path": "Observation.component",
                "sliceName": "slice-b",
                "min": 0,
                "max": "1"
              },
              {
                "id": "Observation.component:slice-b.code",
                "path": "Observation.component.code",
                "sliceName": "slice-b",
                "patternCodeableConcept": {
                  "coding": [
                    {
                      "system": "http://loinc.org",
                      "code": "8480-6"
                    }
                  ]
                }
              }
            ]
          }
        }
        "#;
    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");
    let observation_json = r#"
        {
          "resourceType": "Observation",
          "status": "final",
          "code": { "text": "Blood pressure panel" },
          "component": [
            {
              "code": {
                "coding": [
                  { "system": "http://loinc.org", "code": "8480-6" }
                ]
              }
            }
          ]
        }
        "#;
    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");
    let issues = validate_slicing(&observation, "Observation", &extracted);
    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component[0]")
    );
    assert!(issues.iter().any(|i| i.code == "structure"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "Element 'Observation.component[0]' matches multiple declared slices on 'Observation.component': slice-a, slice-b."
        )
    }));
}
#[test]
fn type_discriminator_matches_nested_resource_type_for_parameter_slices() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "parameters-resource-slicing",
      "url": "http://atrius.health/fhir/StructureDefinition/parameters-resource-slicing",
      "name": "ParametersResourceSlicing",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Parameters",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Parameters",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Parameters.parameter",
            "path": "Parameters.parameter",
            "slicing": {
              "discriminator": [
                { "type": "type", "path": "resource" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Parameters.parameter:obs",
            "path": "Parameters.parameter",
            "sliceName": "obs",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:obs.resource",
            "path": "Parameters.parameter.resource",
            "sliceName": "obs",
            "type": [
              { "code": "Observation" }
            ]
          },
          {
            "id": "Parameters.parameter:patient",
            "path": "Parameters.parameter",
            "sliceName": "patient",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:patient.resource",
            "path": "Parameters.parameter.resource",
            "sliceName": "patient",
            "type": [
              { "code": "Patient" }
            ]
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let parameters_json = r#"
    {
      "resourceType": "Parameters",
      "parameter": [
        {
          "name": "payload1",
          "resource": {
            "resourceType": "Observation",
            "status": "final",
            "code": { "text": "Example observation" }
          }
        },
        {
          "name": "payload2",
          "resource": {
            "resourceType": "Patient"
          }
        }
      ]
    }
    "#;

    let parameters: serde_json::Value = serde_json::from_str(parameters_json)
        .expect("Parameters JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&parameters, "Parameters", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn position_discriminator_matches_items_in_declared_slice_positions() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "parameters-position-slicing",
      "url": "http://atrius.health/fhir/StructureDefinition/parameters-position-slicing",
      "name": "ParametersPositionSlicing",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Parameters",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Parameters",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Parameters.parameter",
            "path": "Parameters.parameter",
            "slicing": {
              "discriminator": [
                { "type": "position", "path": "" }
              ],
              "ordered": true,
              "rules": "closed"
            }
          },
          {
            "id": "Parameters.parameter:first",
            "path": "Parameters.parameter",
            "sliceName": "first",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:second",
            "path": "Parameters.parameter",
            "sliceName": "second",
            "min": 1,
            "max": "1"
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let parameters_json = r#"
    {
      "resourceType": "Parameters",
      "parameter": [
        {
          "name": "p1"
        },
        {
          "name": "p2"
        }
      ]
    }
    "#;

    let parameters: serde_json::Value = serde_json::from_str(parameters_json)
        .expect("Parameters JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&parameters, "Parameters", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn position_discriminator_rejects_missing_required_position_slice() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "parameters-position-slicing-missing",
      "url": "http://atrius.health/fhir/StructureDefinition/parameters-position-slicing-missing",
      "name": "ParametersPositionSlicingMissing",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Parameters",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Parameters",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Parameters.parameter",
            "path": "Parameters.parameter",
            "slicing": {
              "discriminator": [
                { "type": "position", "path": "" }
              ],
              "ordered": true,
              "rules": "closed"
            }
          },
          {
            "id": "Parameters.parameter:first",
            "path": "Parameters.parameter",
            "sliceName": "first",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:second",
            "path": "Parameters.parameter",
            "sliceName": "second",
            "min": 1,
            "max": "1"
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let parameters_json = r#"
    {
      "resourceType": "Parameters",
      "parameter": [
        {
          "name": "only-one"
        }
      ]
    }
    "#;

    let parameters: serde_json::Value = serde_json::from_str(parameters_json)
        .expect("Parameters JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&parameters, "Parameters", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Parameters.parameter:second")
    );
    assert!(issues.iter().any(|i| i.code == "required"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "Slice 'Parameters.parameter:second' requires at least 1 occurrence(s), but found 0.",
        )
    }));
}

#[test]
fn type_discriminator_matches_reference_shaped_value() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "observation-focus-reference-slicing",
      "url": "http://atrius.health/fhir/StructureDefinition/observation-focus-reference-slicing",
      "name": "ObservationFocusReferenceSlicing",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.focus",
            "path": "Observation.focus",
            "slicing": {
              "discriminator": [
                { "type": "type", "path": "" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Observation.focus:ref",
            "path": "Observation.focus",
            "sliceName": "ref",
            "min": 1,
            "max": "1",
            "type": [
              { "code": "Reference" }
            ]
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Example" },
      "focus": [
        {
          "reference": "Patient/123",
          "display": "Example patient"
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn type_discriminator_matches_codeable_reference_shaped_value() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "observation-focus-codeable-reference-slicing",
      "url": "http://atrius.health/fhir/StructureDefinition/observation-focus-codeable-reference-slicing",
      "name": "ObservationFocusCodeableReferenceSlicing",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.focus",
            "path": "Observation.focus",
            "slicing": {
              "discriminator": [
                { "type": "type", "path": "" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Observation.focus:codeable-ref",
            "path": "Observation.focus",
            "sliceName": "codeable-ref",
            "min": 1,
            "max": "1",
            "type": [
              { "code": "CodeableReference" }
            ]
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Example" },
      "focus": [
        {
          "reference": "Patient/123",
          "display": "Example patient"
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}
#[test]
fn type_discriminator_matches_choice_value_x_concrete_json_keys() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "parameters-valuex-slicing",
      "url": "http://atrius.health/fhir/StructureDefinition/parameters-valuex-slicing",
      "name": "ParametersValueXSlicing",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Parameters",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Parameters",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Parameters.parameter",
            "path": "Parameters.parameter",
            "slicing": {
              "discriminator": [
                { "type": "type", "path": "value[x]" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Parameters.parameter:int",
            "path": "Parameters.parameter",
            "sliceName": "int",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:int.value[x]",
            "path": "Parameters.parameter.value[x]",
            "sliceName": "int",
            "type": [
              { "code": "integer" }
            ]
          },
          {
            "id": "Parameters.parameter:str",
            "path": "Parameters.parameter",
            "sliceName": "str",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:str.value[x]",
            "path": "Parameters.parameter.value[x]",
            "sliceName": "str",
            "type": [
              { "code": "string" }
            ]
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let parameters_json = r#"
    {
      "resourceType": "Parameters",
      "parameter": [
        {
          "name": "n",
          "valueInteger": 1
        },
        {
          "name": "s",
          "valueString": "hello"
        }
      ]
    }
    "#;

    let parameters: serde_json::Value = serde_json::from_str(parameters_json)
        .expect("Parameters JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&parameters, "Parameters", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn exists_discriminator_matches_present_and_absent_optional_child() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "observation-component-exists-slicing",
      "url": "http://atrius.health/fhir/StructureDefinition/observation-component-exists-slicing",
      "name": "ObservationComponentExistsSlicing",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "exists", "path": "valueString" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Observation.component:with-value",
            "path": "Observation.component",
            "sliceName": "with-value",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:with-value.valueString",
            "path": "Observation.component.valueString",
            "sliceName": "with-value",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:without-value",
            "path": "Observation.component",
            "sliceName": "without-value",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:without-value.valueString",
            "path": "Observation.component.valueString",
            "sliceName": "without-value",
            "min": 0,
            "max": "0"
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Example" },
      "component": [
        {
          "valueString": "present"
        },
        {
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn exists_discriminator_rejects_item_when_no_slice_matches_presence_state() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "observation-component-exists-slicing-no-absent-slice",
      "url": "http://atrius.health/fhir/StructureDefinition/observation-component-exists-slicing-no-absent-slice",
      "name": "ObservationComponentExistsSlicingNoAbsentSlice",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "exists", "path": "valueString" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Observation.component:with-value",
            "path": "Observation.component",
            "sliceName": "with-value",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:with-value.valueString",
            "path": "Observation.component.valueString",
            "sliceName": "with-value",
            "min": 1,
            "max": "1"
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Example" },
      "component": [
        {
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component[0]")
    );
    assert!(issues.iter().any(|i| i.code == "structure"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "does not match any declared slice on 'Observation.component', and slicing rules are closed"
        )
    }));
}

#[test]
fn type_discriminator_choice_value_x_reports_unmatched_wrong_choice_type() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "parameters-valuex-slicing-wrong-type",
      "url": "http://atrius.health/fhir/StructureDefinition/parameters-valuex-slicing-wrong-type",
      "name": "ParametersValueXSlicingWrongType",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Parameters",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Parameters",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Parameters.parameter",
            "path": "Parameters.parameter",
            "slicing": {
              "discriminator": [
                { "type": "type", "path": "value[x]" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Parameters.parameter:int",
            "path": "Parameters.parameter",
            "sliceName": "int",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:int.value[x]",
            "path": "Parameters.parameter.value[x]",
            "sliceName": "int",
            "type": [
              { "code": "integer" }
            ]
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let parameters_json = r#"
    {
      "resourceType": "Parameters",
      "parameter": [
        {
          "name": "wrong",
          "valueString": "oops"
        }
      ]
    }
    "#;

    let parameters: serde_json::Value = serde_json::from_str(parameters_json)
        .expect("Parameters JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&parameters, "Parameters", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Parameters.parameter[0]")
    );
    assert!(issues.iter().any(|i| i.code == "structure"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "does not match any declared slice on 'Parameters.parameter', and slicing rules are closed"
        )
    }));
    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Parameters.parameter:int" && i.code == "required")
    );
}

#[test]
fn open_at_end_allows_unmatched_items_only_at_the_end() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile-open-at-end",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile-open-at-end",
      "name": "BloodPressurePanelProfileOpenAtEnd",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": true,
              "rules": "openAtEnd"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8480-6"
                }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8480-6" }
            ]
          }
        },
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "9999-9" }
            ]
          }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn open_at_end_rejects_declared_slice_after_unmatched_tail_item() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile-open-at-end-reject",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile-open-at-end-reject",
      "name": "BloodPressurePanelProfileOpenAtEndReject",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": false,
              "rules": "openAtEnd"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                {
                  "system": "http://loinc.org",
                  "code": "8480-6"
                }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "9999-9" }
            ]
          }
        },
        {
          "code": {
            "coding": [
              { "system": "http://loinc.org", "code": "8480-6" }
            ]
          }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component[1]")
    );
    assert!(issues.iter().any(|i| i.code == "structure"));
    assert!(issues.iter().any(|i| {
        i.diagnostics.contains(
            "but openAtEnd requires all unmatched content to appear only after the declared slices",
        )
    }));
}
#[test]
fn ordered_slicing_accepts_declared_slices_in_definition_order() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile-ordered-ok",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile-ordered-ok",
      "name": "BloodPressurePanelProfileOrderedOk",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": true,
              "rules": "open"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                { "system": "http://loinc.org", "code": "8480-6" }
              ]
            }
          },
          {
            "id": "Observation.component:diastolic",
            "path": "Observation.component",
            "sliceName": "diastolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:diastolic.code",
            "path": "Observation.component.code",
            "sliceName": "diastolic",
            "patternCodeableConcept": {
              "coding": [
                { "system": "http://loinc.org", "code": "8462-4" }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": { "coding": [ { "system": "http://loinc.org", "code": "8480-6" } ] }
        },
        {
          "code": { "coding": [ { "system": "http://loinc.org", "code": "8462-4" } ] }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn ordered_slicing_rejects_declared_slices_out_of_definition_order() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile-ordered-bad",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile-ordered-bad",
      "name": "BloodPressurePanelProfileOrderedBad",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": true,
              "rules": "open"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                { "system": "http://loinc.org", "code": "8480-6" }
              ]
            }
          },
          {
            "id": "Observation.component:diastolic",
            "path": "Observation.component",
            "sliceName": "diastolic",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Observation.component:diastolic.code",
            "path": "Observation.component.code",
            "sliceName": "diastolic",
            "patternCodeableConcept": {
              "coding": [
                { "system": "http://loinc.org", "code": "8462-4" }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": { "coding": [ { "system": "http://loinc.org", "code": "8462-4" } ] }
        },
        {
          "code": { "coding": [ { "system": "http://loinc.org", "code": "8480-6" } ] }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(
        issues
            .iter()
            .any(|i| i.fhir_path == "Observation.component[1]")
    );
    assert!(issues.iter().any(|i| i.code == "structure"));
    assert!(issues.iter().any(|i| {
        i.diagnostics
            .contains("but ordered slicing requires slices to appear in declaration order")
    }));
}

#[test]
fn open_at_end_without_ordered_emits_warning() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bp-panel-profile-open-at-end-unordered",
      "url": "http://atrius.health/fhir/StructureDefinition/bp-panel-profile-open-at-end-unordered",
      "name": "BloodPressurePanelProfileOpenAtEndUnordered",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Observation",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Observation",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Observation.component",
            "path": "Observation.component",
            "slicing": {
              "discriminator": [
                { "type": "value", "path": "code" }
              ],
              "ordered": false,
              "rules": "openAtEnd"
            }
          },
          {
            "id": "Observation.component:systolic",
            "path": "Observation.component",
            "sliceName": "systolic",
            "min": 0,
            "max": "1"
          },
          {
            "id": "Observation.component:systolic.code",
            "path": "Observation.component.code",
            "sliceName": "systolic",
            "patternCodeableConcept": {
              "coding": [
                { "system": "http://loinc.org", "code": "8480-6" }
              ]
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json)
        .expect("StructureDefinition JSON should deserialize into R5 StructureDefinition");
    let extracted: ExtractedProfile = extract_r5_structure_definition_profile(&sd)
        .expect("StructureDefinition extraction should succeed");

    let observation_json = r#"
    {
      "resourceType": "Observation",
      "status": "final",
      "code": { "text": "Blood pressure panel" },
      "component": [
        {
          "code": { "coding": [ { "system": "http://loinc.org", "code": "8480-6" } ] }
        },
        {
          "code": { "coding": [ { "system": "http://loinc.org", "code": "9999-9" } ] }
        }
      ]
    }
    "#;

    let observation: serde_json::Value = serde_json::from_str(observation_json)
        .expect("Observation JSON should deserialize into raw JSON Value");

    let issues = validate_slicing(&observation, "Observation", &extracted);

    assert!(issues.iter().any(|i| i.code == "business-rule"));
    assert!(
        issues
            .iter()
            .any(|i| { i.diagnostics.contains("uses openAtEnd but is not ordered") })
    );
}
#[test]
fn profile_discriminator_matches_declared_meta_profile_on_nested_resource() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "parameters-profile-slicing",
      "url": "http://atrius.health/fhir/StructureDefinition/parameters-profile-slicing",
      "name": "ParametersProfileSlicing",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Parameters",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Parameters",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Parameters.parameter",
            "path": "Parameters.parameter",
            "slicing": {
              "discriminator": [
                { "type": "profile", "path": "resource" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Parameters.parameter:obs",
            "path": "Parameters.parameter",
            "sliceName": "obs",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:obs.resource",
            "path": "Parameters.parameter.resource",
            "sliceName": "obs",
            "type": [
              {
                "code": "Observation",
                "profile": ["http://atrius.health/fhir/StructureDefinition/obs-profile"]
              }
            ]
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json).unwrap();
    let extracted = extract_r5_structure_definition_profile(&sd).unwrap();

    let parameters_json = r#"
    {
      "resourceType": "Parameters",
      "parameter": [
        {
          "name": "p",
          "resource": {
            "resourceType": "Observation",
            "meta": {
              "profile": ["http://atrius.health/fhir/StructureDefinition/obs-profile"]
            }
          }
        }
      ]
    }
    "#;

    let parameters: serde_json::Value = serde_json::from_str(parameters_json).unwrap();
    let issues = validate_slicing(&parameters, "Parameters", &extracted);

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn profile_discriminator_falls_back_to_validation_when_no_meta_profile_present() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "parameters-profile-slicing-fallback",
      "url": "http://atrius.health/fhir/StructureDefinition/parameters-profile-slicing-fallback",
      "name": "ParametersProfileSlicingFallback",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Parameters",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Parameters",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Parameters.parameter",
            "path": "Parameters.parameter",
            "slicing": {
              "discriminator": [
                { "type": "profile", "path": "resource" }
              ],
              "ordered": false,
              "rules": "closed"
            }
          },
          {
            "id": "Parameters.parameter:obs",
            "path": "Parameters.parameter",
            "sliceName": "obs",
            "min": 1,
            "max": "1"
          },
          {
            "id": "Parameters.parameter:obs.resource",
            "path": "Parameters.parameter.resource",
            "sliceName": "obs",
            "type": [
              {
                "code": "Observation",
                "profile": ["http://atrius.health/fhir/StructureDefinition/obs-profile"]
              }
            ]
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json).unwrap();
    let extracted = extract_r5_structure_definition_profile(&sd).unwrap();

    let parameters_json = r#"
    {
      "resourceType": "Parameters",
      "parameter": [
        {
          "name": "p",
          "resource": {
            "resourceType": "Observation"
          }
        }
      ]
    }
    "#;

    let parameters: serde_json::Value = serde_json::from_str(parameters_json).unwrap();
    let extracted_profile_map = std::collections::HashMap::from([(
        "http://atrius.health/fhir/StructureDefinition/obs-profile".to_string(),
        ExtractedProfile {
            url: "http://atrius.health/fhir/StructureDefinition/obs-profile".to_string(),
            version: None,
            name: Some("ObsProfile".to_string()),
            title: None,
            resource_type: "Observation".to_string(),
            base_definition: None,
            element_rules: vec![],
            invariants: vec![],
            ..Default::default()
        },
    )]);
    let config = ValidationConfig::default();
    let ctx = fhir_validation::profile::validate::ValidationContext {
        fhir_version: FhirVersion::R5,
        validator: &fhir_validation::Validator::new(config),
        terminology: None,
        evaluator: &StubFhirPathEvaluator,
        runtime_profile_registry: None,
        extracted_profile_map: &extracted_profile_map,
    };

    let mut state = fhir_validation::profile::validate::ValidationState::default();
    let issues = validate_slicing_with_context(
        Some(&ctx),
        &mut state,
        &parameters,
        "Parameters",
        &extracted,
    );

    assert!(
        issues.is_empty(),
        "Expected no slicing issues, got: {issues:#?}"
    );
}

#[test]
fn profile_discriminator_with_resolve_path_emits_warning() {
    let sd_json = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "parameters-profile-resolve",
      "url": "http://atrius.health/fhir/StructureDefinition/parameters-profile-resolve",
      "name": "ParametersProfileResolve",
      "status": "draft",
      "kind": "resource",
      "abstract": false,
      "type": "Parameters",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Parameters",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Parameters.parameter",
            "path": "Parameters.parameter",
            "slicing": {
              "discriminator": [
                { "type": "profile", "path": "resource.resolve()" }
              ],
              "ordered": false,
              "rules": "open"
            }
          }
        ]
      }
    }
    "#;

    let sd: StructureDefinition = serde_json::from_str(sd_json).unwrap();
    let extracted = extract_r5_structure_definition_profile(&sd).unwrap();

    let parameters_json = r#"
    {
      "resourceType": "Parameters",
      "parameter": []
    }
    "#;

    let parameters: serde_json::Value = serde_json::from_str(parameters_json).unwrap();
    let issues = validate_slicing(&parameters, "Parameters", &extracted);

    assert!(issues.iter().any(|i| i.code == "business-rule"));
}
