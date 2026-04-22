mod tests {
    use crate::common::fixtures::{
        assert_has_binding_issue, assert_has_error, load_resource, local_terminology_r4,
    };
    use fhir_validation::binding::common::BindingCheckContextSync;
    use fhir_validation::r4::binding::validate_primitive_code_binding;
    use fhir_validation::{LocalTerminologyService, R4FhirPathEvaluator};
    use fhir_validation::{ValidationConfig, Validator};
    use fhir_validation_types::{BindingStrength, Severity};
    use helios_fhir::{FhirResource, FhirVersion, TerminologyValidationError};
    pub fn r4_evaluator_for(resource: &FhirResource) -> R4FhirPathEvaluator {
        let FhirResource::R4(r) = resource else {
            panic!("expected R4 resource");
        };
        R4FhirPathEvaluator::new((**r).clone())
    }
    fn validator() -> Validator {
        Validator::new(ValidationConfig::default())
    }

    #[test]
    fn absent_code_produces_no_issue() {
        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.gender",
            "http://hl7.org/fhir/ValueSet/administrative-gender",
            BindingStrength::Required,
            None,
        );
        let issues = validate_primitive_code_binding(&ctx, None, None, |_| Ok(()));

        assert!(issues.is_empty());
    }

    #[test]
    fn local_success_produces_no_issue() {
        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.gender",
            "http://hl7.org/fhir/ValueSet/administrative-gender",
            BindingStrength::Required,
            None,
        );
        let issues = validate_primitive_code_binding(&ctx, Some("male"), None, |_| Ok(()));

        assert!(issues.is_empty());
    }

    #[test]
    fn remote_false_produces_warning_for_extensible_binding() {
        let term = LocalTerminologyService::new(FhirVersion::R4);

        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.language",
            "http://hl7.org/fhir/ValueSet/languages",
            BindingStrength::Extensible,
            Some(&term),
        );
        let issues =
            validate_primitive_code_binding(&ctx, Some("xx"), Some("remote required"), |_| {
                Err(TerminologyValidationError::RemoteValidationRequired(
                    "remote required".to_string(),
                ))
            });

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].code, "terminology");
        assert_eq!(
            issues[0].detail_code,
            Some(fhir_validation::ValidationIssueDetailCode::TerminologyValidationFailed)
        );
    }

    #[test]
    fn remote_required_without_service_produces_terminology_error() {
        let v = validator();
        let ctx = BindingCheckContextSync::new(
            &v,
            "Patient.language",
            "http://hl7.org/fhir/ValueSet/languages",
            BindingStrength::Required,
            None,
        );
        let issues =
            validate_primitive_code_binding(&ctx, Some("en"), Some("remote required"), |_| {
                Err(TerminologyValidationError::RemoteValidationRequired(
                    "remote required".to_string(),
                ))
            });

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert_eq!(issues[0].code, "terminology");
    }
    #[test]
    fn r4_patient_invalid_gender() {
        let resource = load_resource(
            FhirVersion::R4,
            "invalid/patient/patient-invalid-gender.json",
        );
        let validator = Validator::default();
        let evaluator = r4_evaluator_for(&resource);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&resource, Some(&term), &evaluator);
        // println!("{:#?}", issues);
        assert_has_binding_issue(
            &issues,
            "Patient.gender",
            "http://hl7.org/fhir/ValueSet/administrative-gender|4.0.1",
        );
        assert_has_error(&issues);
    }
}
