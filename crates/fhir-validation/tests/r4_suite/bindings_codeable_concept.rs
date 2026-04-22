mod tests {
    use fhir_validation::LocalTerminologyService;
    use fhir_validation::binding::common::BindingCheckContextSync;
    use fhir_validation::r4::binding::validate_codeable_concept_binding;
    use fhir_validation::{ValidationConfig, Validator};
    use fhir_validation_types::{BindingStrength, Severity};
    use helios_fhir::Element;
    use helios_fhir::FhirVersion;
    use helios_fhir::TerminologyValidationError;
    use helios_fhir::r4::{Code, CodeableConcept, Coding, Uri};

    fn validator() -> Validator {
        Validator::new(ValidationConfig::default())
    }

    fn code(value: &str) -> Code {
        Element {
            id: None,
            extension: None,
            value: Some(value.to_string()),
        }
    }

    fn uri(value: &str) -> Uri {
        Element {
            id: None,
            extension: None,
            value: Some(value.to_string()),
        }
    }

    fn coding(system: &str, code_value: &str, display: Option<&str>) -> Coding {
        Coding {
            id: None,
            extension: None,
            system: Some(uri(system)),
            version: None,
            code: Some(code(code_value)),
            display: display.map(|d| Element {
                id: None,
                extension: None,
                value: Some(d.to_string()),
            }),
            user_selected: None,
        }
    }

    fn cc_with_one_coding(system: &str, code_value: &str) -> CodeableConcept {
        CodeableConcept {
            id: None,
            extension: None,
            coding: Some(vec![coding(system, code_value, None)]),
            text: None,
        }
    }

    #[test]
    fn absent_codeable_concept_produces_no_issue() {
        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.maritalStatus",
            "http://hl7.org/fhir/ValueSet/marital-status",
            BindingStrength::Extensible,
            None,
        );
        let issues = validate_codeable_concept_binding(&ctx, None, |_| Ok(()));

        assert!(issues.is_empty());
    }

    #[test]
    fn local_success_produces_no_issue() {
        let cc = cc_with_one_coding("http://terminology.hl7.org/CodeSystem/v3-NullFlavor", "UNK");

        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.maritalStatus",
            "http://hl7.org/fhir/ValueSet/marital-status",
            BindingStrength::Extensible,
            None,
        );
        let issues = validate_codeable_concept_binding(&ctx, Some(&cc), |_| Ok(()));

        assert!(issues.is_empty());
    }

    #[test]
    fn remote_true_accepts_if_any_coding_matches() {
        let cc = CodeableConcept {
            id: None,
            extension: None,
            coding: Some(vec![
                coding("http://example.org/system", "bad", None),
                coding("http://hl7.org/fhir/administrative-gender", "male", None),
            ]),
            text: None,
        };

        let term = LocalTerminologyService::new(FhirVersion::R4);

        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.gender",
            "http://hl7.org/fhir/ValueSet/administrative-gender",
            BindingStrength::Extensible,
            Some(&term),
        );
        let issues = validate_codeable_concept_binding(&ctx, Some(&cc), |_| {
            Err(TerminologyValidationError::RemoteValidationRequired(
                "remote required".to_string(),
            ))
        });
        assert!(issues.is_empty());
    }

    #[test]
    fn remote_false_produces_warning_for_extensible_binding() {
        let cc = cc_with_one_coding("http://example.org/system", "X");
        let term = LocalTerminologyService::new(FhirVersion::R4);

        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.maritalStatus",
            "http://hl7.org/fhir/ValueSet/marital-status",
            BindingStrength::Extensible,
            Some(&term),
        );
        let issues = validate_codeable_concept_binding(&ctx, Some(&cc), |_| {
            Err(TerminologyValidationError::RemoteValidationRequired(
                "remote required".to_string(),
            ))
        });

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].code, "terminology");
    }

    #[test]
    fn remote_required_without_service_produces_terminology_error() {
        let cc = cc_with_one_coding("http://example.org/system", "X");

        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.maritalStatus",
            "http://hl7.org/fhir/ValueSet/marital-status",
            BindingStrength::Required,
            None,
        );
        let issues = validate_codeable_concept_binding(&ctx, Some(&cc), |_| {
            Err(TerminologyValidationError::RemoteValidationRequired(
                "remote required".to_string(),
            ))
        });
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert_eq!(issues[0].code, "terminology");
    }
}
