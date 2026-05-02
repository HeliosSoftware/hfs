mod tests {

    use fhir_validation::profile::extract::extract_r5_structure_definition_profile;
    use fhir_validation::profile::structure_definition_extract::StructureDefinitionExtractMessage;
    use fhir_validation::{StructureDefinitionKind, TypeDerivationRule, ValidationError};
    use fhir_validation_types::BindingTargetKind;
    use helios_fhir::r5::StructureDefinition;

    const ATRIUS_PATIENT_PROFILE_JSON: &str = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "atrius-patient",
      "url": "http://atrius.health/fhir/StructureDefinition/atrius-patient",
      "version": "0.1.0",
      "name": "AtriusPatient",
      "title": "Atrius Patient Profile",
      "status": "draft",
      "date": "2026-01-01",
      "publisher": "Atrius Health",
      "kind": "resource",
      "abstract": false,
      "type": "Patient",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Patient",
            "path": "Patient",
            "constraint": [
              {
                "key": "atrius-pat-1",
                "severity": "error",
                "human": "If patient is active, at least one name must be present",
                "expression": "active = true implies name.exists()"
              }
            ]
          },
          {
            "id": "Patient.identifier",
            "path": "Patient.identifier",
            "min": 1
          },
          {
            "id": "Patient.gender",
            "path": "Patient.gender",
            "min": 1,
            "binding": {
              "extension": [
                {
                  "url": "http://hl7.org/fhir/StructureDefinition/elementdefinition-bindingName",
                  "valueString": "AdministrativeGender"
                }
              ],
              "strength": "required",
              "valueSet": "http://hl7.org/fhir/ValueSet/administrative-gender"
            }
          },
          {
            "id": "Patient.maritalStatus",
            "path": "Patient.maritalStatus",
            "type": [{ "code": "CodeableConcept" }],
            "binding": {
              "strength": "required",
              "valueSet": "http://hl7.org/fhir/ValueSet/marital-status"
            }
          },
          {
            "id": "Patient.birthDate",
            "path": "Patient.birthDate",
            "min": 1
          }
        ]
      }
    }
    "#;

    /// Differential omits `type` on `Patient.maritalStatus`; snapshot supplies
    /// `CodeableConcept` so binding target kind is not inferred as `Code`.
    const SNAPSHOT_RESOLVES_TYPE_JSON: &str = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "snapshot-marital-demo",
      "url": "http://example.org/fhir/StructureDefinition/snapshot-marital-demo",
      "name": "SnapshotMaritalDemo",
      "kind": "resource",
      "type": "Patient",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
      "derivation": "constraint",
      "snapshot": {
        "element": [
          {
            "id": "Patient",
            "path": "Patient",
            "definition": "A patient",
            "min": 0,
            "max": "*"
          },
          {
            "id": "Patient.maritalStatus",
            "path": "Patient.maritalStatus",
            "definition": "Marital status",
            "min": 0,
            "max": "1",
            "type": [{ "code": "CodeableConcept" }],
            "binding": {
              "strength": "required",
              "valueSet": "http://hl7.org/fhir/ValueSet/marital-status"
            }
          }
        ]
      },
      "differential": {
        "element": [
          {
            "id": "Patient",
            "path": "Patient",
            "constraint": [
              {
                "key": "snap-demo-1",
                "severity": "error",
                "human": "Demo",
                "expression": "true"
              }
            ]
          },
          {
            "id": "Patient.maritalStatus",
            "path": "Patient.maritalStatus",
            "binding": {
              "strength": "required",
              "valueSet": "http://hl7.org/fhir/ValueSet/marital-status"
            }
          }
        ]
      }
    }
    "#;

    /// Snapshot-first extraction should include constrained paths that are only
    /// present in snapshot and omitted from differential.
    const SNAPSHOT_ONLY_PATH_JSON: &str = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "snapshot-only-path-demo",
      "url": "http://example.org/fhir/StructureDefinition/snapshot-only-path-demo",
      "name": "SnapshotOnlyPathDemo",
      "kind": "resource",
      "type": "Patient",
      "baseDefinition": "http://example.org/fhir/StructureDefinition/abdm-patient",
      "derivation": "constraint",
      "snapshot": {
        "element": [
          {
            "id": "Patient",
            "path": "Patient"
          },
          {
            "id": "Patient.name",
            "path": "Patient.name",
            "min": 1
          }
        ]
      },
      "differential": {
        "element": [
          {
            "id": "Patient",
            "path": "Patient"
          }
        ]
      }
    }
    "#;

    /// Minimal primitive specialization: `kind` + `derivation` + one differential row.
    const SPECIALIZATION_PRIMITIVE_STRING_JSON: &str = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "special-string",
      "url": "http://example.org/fhir/StructureDefinition/special-string",
      "name": "SpecialString",
      "status": "draft",
      "kind": "primitive-type",
      "abstract": false,
      "type": "string",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/string",
      "derivation": "specialization",
      "differential": {
        "element": [
          {
            "id": "string",
            "path": "string",
            "min": 0,
            "max": "*"
          }
        ]
      }
    }
    "#;

    /// `maxLength`, `minValue`/`maxValue`, `mustSupport`, `isModifier` (+ reason).
    const BOUNDS_AND_FLAGS_META_JSON: &str = r#"
    {
      "resourceType": "StructureDefinition",
      "id": "bounds-demo",
      "url": "http://example.org/fhir/StructureDefinition/bounds-demo",
      "name": "BoundsDemo",
      "status": "draft",
      "kind": "resource",
      "type": "Patient",
      "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
      "derivation": "constraint",
      "differential": {
        "element": [
          {
            "id": "Patient",
            "path": "Patient",
            "constraint": [
              {
                "key": "bd-1",
                "severity": "error",
                "human": "demo",
                "expression": "true"
              }
            ]
          },
          {
            "id": "Patient.name.family",
            "path": "Patient.name.family",
            "maxLength": 50
          },
          {
            "id": "Patient.birthDate",
            "path": "Patient.birthDate",
            "minValueDate": "1990-01-01",
            "maxValueDate": "2010-12-31"
          },
          {
            "id": "Patient.active",
            "path": "Patient.active",
            "mustSupport": true,
            "isModifier": true,
            "isModifierReason": "Status affects interpretation"
          }
        ]
      }
    }
    "#;

    #[test]
    fn extracts_atrius_patient_profile() {
        let sd: StructureDefinition = serde_json::from_str(ATRIUS_PATIENT_PROFILE_JSON)
            .expect("Atrius Patient profile JSON should deserialize into R5 StructureDefinition");

        let profile = extract_r5_structure_definition_profile(&sd)
            .expect("Atrius Patient profile should extract successfully");

        assert_eq!(
            profile.url,
            "http://atrius.health/fhir/StructureDefinition/atrius-patient"
        );
        assert_eq!(profile.version.as_deref(), Some("0.1.0"));
        assert_eq!(profile.name.as_deref(), Some("AtriusPatient"));
        assert_eq!(profile.title.as_deref(), Some("Atrius Patient Profile"));
        assert_eq!(profile.resource_type, "Patient");
        assert_eq!(
            profile.base_definition.as_deref(),
            Some("http://hl7.org/fhir/StructureDefinition/Patient")
        );
        assert_eq!(profile.kind, StructureDefinitionKind::Resource);
        assert_eq!(profile.derivation, TypeDerivationRule::Constraint);

        assert_eq!(profile.invariants.len(), 1);
        let invariant = &profile.invariants[0];
        assert_eq!(invariant.key, "atrius-pat-1");
        assert_eq!(invariant.path, "Patient");
        assert_eq!(
            invariant.human,
            "If patient is active, at least one name must be present"
        );
        assert_eq!(invariant.expression, "active = true implies name.exists()");

        assert_eq!(profile.element_rules.len(), 4);

        let identifier_rule = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.identifier")
            .expect("Patient.identifier rule should be extracted");
        assert_eq!(identifier_rule.id, "Patient.identifier");
        assert_eq!(identifier_rule.min, Some(1));
        assert!(identifier_rule.binding.is_none());
        assert!(identifier_rule.constraints.is_empty());

        let gender_rule = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.gender")
            .expect("Patient.gender rule should be extracted");
        assert_eq!(gender_rule.id, "Patient.gender");
        assert_eq!(gender_rule.min, Some(1));
        let binding = gender_rule
            .binding
            .as_ref()
            .expect("Patient.gender binding should be extracted");
        assert_eq!(binding.path, "Patient.gender");
        assert_eq!(
            binding.value_set,
            "http://hl7.org/fhir/ValueSet/administrative-gender"
        );
        assert_eq!(
            binding.binding_name.as_deref(),
            Some("AdministrativeGender")
        );
        assert_eq!(
            binding.target_kind,
            BindingTargetKind::Code,
            "no declared type in differential: legacy fallback treats binding as primitive code"
        );
        assert!(gender_rule.constraints.is_empty());

        let marital_rule = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.maritalStatus")
            .expect("Patient.maritalStatus rule should be extracted");
        let ms_binding = marital_rule
            .binding
            .as_ref()
            .expect("Patient.maritalStatus binding should be extracted");
        assert_eq!(ms_binding.target_kind, BindingTargetKind::CodeableConcept);

        let birth_date_rule = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.birthDate")
            .expect("Patient.birthDate rule should be extracted");
        assert_eq!(birth_date_rule.id, "Patient.birthDate");
        assert_eq!(birth_date_rule.min, Some(1));
        assert!(birth_date_rule.binding.is_none());
        assert!(birth_date_rule.constraints.is_empty());
    }

    #[test]
    fn differential_resolves_against_snapshot_for_missing_type() {
        let sd: StructureDefinition = serde_json::from_str(SNAPSHOT_RESOLVES_TYPE_JSON)
            .expect("demo StructureDefinition JSON should deserialize");

        let profile = extract_r5_structure_definition_profile(&sd)
            .expect("profile with snapshot should extract");

        let marital = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.maritalStatus")
            .expect("maritalStatus rule");
        let binding = marital.binding.as_ref().expect("maritalStatus binding");
        assert_eq!(binding.target_kind, BindingTargetKind::CodeableConcept);
    }

    #[test]
    fn snapshot_first_extracts_paths_not_present_in_differential() {
        let sd: StructureDefinition = serde_json::from_str(SNAPSHOT_ONLY_PATH_JSON)
            .expect("snapshot-only-path StructureDefinition JSON should deserialize");

        let profile = extract_r5_structure_definition_profile(&sd)
            .expect("profile with snapshot-only constrained path should extract");

        let name_rule = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.name")
            .expect("Patient.name rule should be extracted from snapshot");
        assert_eq!(name_rule.min, Some(1));
    }

    #[test]
    fn rejects_empty_snapshot_element_when_snapshot_is_present() {
        let mut json: serde_json::Value =
            serde_json::from_str(ATRIUS_PATIENT_PROFILE_JSON).expect("parse");
        json["snapshot"] = serde_json::json!({ "element": [] });

        let sd: StructureDefinition = serde_json::from_value(json).expect("deserialize SD");
        let err = extract_r5_structure_definition_profile(&sd).unwrap_err();
        match err {
            ValidationError::InvalidStructureDefinition(
                StructureDefinitionExtractMessage::SnapshotElementNonEmpty,
            ) => {}
            other => panic!("expected SnapshotElementNonEmpty, got {other:?}"),
        }
    }

    #[test]
    fn extracts_max_length_min_max_value_and_modifier_metadata() {
        let sd: StructureDefinition = serde_json::from_str(BOUNDS_AND_FLAGS_META_JSON)
            .expect("bounds demo StructureDefinition JSON should deserialize");

        let profile = extract_r5_structure_definition_profile(&sd)
            .expect("bounds demo profile should extract");

        let family = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.name.family")
            .expect("Patient.name.family rule");
        assert_eq!(family.max_length, Some(50));

        let birth = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.birthDate")
            .expect("Patient.birthDate rule");
        assert!(
            birth
                .min_value
                .as_ref()
                .is_some_and(|v| v.get("minValueDate").is_some())
        );
        assert!(
            birth
                .max_value
                .as_ref()
                .is_some_and(|v| v.get("maxValueDate").is_some())
        );

        let active = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.active")
            .expect("Patient.active rule");
        assert_eq!(active.must_support, Some(true));
        assert_eq!(active.is_modifier, Some(true));
        assert_eq!(
            active.is_modifier_reason.as_deref(),
            Some("Status affects interpretation")
        );
    }

    #[test]
    fn extracts_specialization_and_primitive_type_kind() {
        let sd: StructureDefinition = serde_json::from_str(SPECIALIZATION_PRIMITIVE_STRING_JSON)
            .expect("primitive specialization SD JSON should deserialize");

        let profile = extract_r5_structure_definition_profile(&sd)
            .expect("primitive specialization profile should extract");

        assert_eq!(profile.kind, StructureDefinitionKind::PrimitiveType);
        assert_eq!(profile.derivation, TypeDerivationRule::Specialization);
        assert_eq!(profile.resource_type, "string");
    }

    #[test]
    fn rejects_unknown_structure_definition_kind() {
        let mut json: serde_json::Value =
            serde_json::from_str(ATRIUS_PATIENT_PROFILE_JSON).expect("parse");
        json["kind"] = serde_json::json!("not-a-valid-kind");

        let sd: StructureDefinition = serde_json::from_value(json).expect("deserialize SD");
        let err = extract_r5_structure_definition_profile(&sd).unwrap_err();
        match err {
            ValidationError::InvalidStructureDefinition(
                StructureDefinitionExtractMessage::UnknownKind { value },
            ) => {
                assert_eq!(value, "not-a-valid-kind");
            }
            other => panic!("expected InvalidStructureDefinition, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_derivation() {
        let mut json: serde_json::Value =
            serde_json::from_str(ATRIUS_PATIENT_PROFILE_JSON).expect("parse");
        json["derivation"] = serde_json::json!("invalid-derivation");

        let sd: StructureDefinition = serde_json::from_value(json).expect("deserialize SD");
        let err = extract_r5_structure_definition_profile(&sd).unwrap_err();
        match err {
            ValidationError::InvalidStructureDefinition(
                StructureDefinitionExtractMessage::UnknownDerivation { value },
            ) => {
                assert_eq!(value, "invalid-derivation");
            }
            other => panic!("expected InvalidStructureDefinition, got {other:?}"),
        }
    }

    /// Differential declares `ElementDefinition.type.aggregation` and `versioning`.
    const TYPE_AGGREGATION_VERSIONING_SD: &str = r#"
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
    fn extracts_type_aggregation_and_versioning() {
        let sd: StructureDefinition = serde_json::from_str(TYPE_AGGREGATION_VERSIONING_SD)
            .expect("aggregation SD JSON should deserialize into R5 StructureDefinition");

        let profile = extract_r5_structure_definition_profile(&sd)
            .expect("aggregation profile should extract");

        let rule = profile
            .element_rules
            .iter()
            .find(|r| r.path == "Patient.link.other")
            .expect("Patient.link.other rule should be extracted");

        assert_eq!(rule.type_constraints.len(), 1);
        let tc = &rule.type_constraints[0];
        assert_eq!(tc.code, "Reference");
        assert_eq!(tc.aggregation, vec!["referenced".to_string()]);
        assert_eq!(tc.versioning.as_deref(), Some("independent"));
    }
}
