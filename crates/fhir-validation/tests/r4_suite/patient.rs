mod tests {
    use crate::common::fixtures::load_resource;
    use crate::common::fixtures::{
        assert_has_invariant, assert_issue_count, assert_no_errors, local_terminology_r4,
    };
    use fhir_validation::R4FhirPathEvaluator;
    use helios_fhir::{FhirResource, FhirVersion};
    use helios_fhirpath::evaluator::apply_additive;
    use helios_fhirpath_support::EvaluationResult;

    pub fn r4_evaluator_for(resource: &FhirResource) -> R4FhirPathEvaluator {
        let FhirResource::R4(r) = resource else {
            panic!("expected R4 resource");
        };
        R4FhirPathEvaluator::new((**r).clone())
    }
    #[test]
    fn patient_example_validates_without_issues() {
        let r = load_resource(FhirVersion::R4, "valid/patient/patient-example.json");
        let validator = fhir_validation::Validator::default();
        let evaluator = r4_evaluator_for(&r);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&r, Some(&term), &evaluator);

        assert_no_errors(&issues);
    }

    #[test]
    fn patient_local_reference_without_matching_contained_resource_emits_invariant() {
        let r = load_resource(
            FhirVersion::R4,
            "invalid/patient/patient-bad-contained-reference.json",
        );
        let validator = fhir_validation::Validator::default();
        let evaluator = r4_evaluator_for(&r);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&r, Some(&term), &evaluator);

        assert_issue_count(&issues, 2);
        assert_has_invariant(
            &issues,
            "Patient.managingOrganization",
            "SHALL have a contained resource if a local reference is provided",
        );
    }
    #[test]
    fn patient_local_reference_with_matching_contained_resource() {
        let r = load_resource(
            FhirVersion::R4,
            "valid/patient/patient-local-contained-reference-valid.json",
        );
        let validator = fhir_validation::Validator::default();
        let evaluator = r4_evaluator_for(&r);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&r, Some(&term), &evaluator);

        assert_no_errors(&issues);
    }
    #[ignore]
    #[test]
    fn patient_non_local_reference_passes() {
        let r = load_resource(
            FhirVersion::R4,
            "valid/patient/patient_non_local_reference.json",
        );
        let validator = fhir_validation::Validator::default();
        let evaluator = r4_evaluator_for(&r);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&r, Some(&term), &evaluator);

        assert_no_errors(&issues);
    }
    #[ignore]
    #[test]
    fn patient_malformed_reference() {
        let r = load_resource(
            FhirVersion::R4,
            "invalid/patient/patient_malformed_reference.json",
        );
        let validator = fhir_validation::Validator::default();
        let evaluator = r4_evaluator_for(&r);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&r, Some(&term), &evaluator);
        assert_issue_count(&issues, 1);
        assert_has_invariant(
            &issues,
            "Patient",
            "If the resource is contained in another resource, it SHALL be referred to from elsewhere in the resource or SHALL refer to the containing resource",
        );
    }
    // #[ignore]
    // #[test]
    // fn debug_patient_malformed_reference_subexpressions() {
    //     let patient = load_r4_patient("invalid/patient/patient_malformed_reference.json");
    //
    //     let exprs = [
    //         "%resource.descendants().reference",
    //         "%resource.contained.id",
    //         "'#' + %resource.contained.first().id",
    //         "'#' + %resource.contained.first().id in %resource.descendants().reference",
    //         "contained.where((('#'+id in (%resource.descendants().reference | %resource.descendants().ofType(canonical) | %resource.descendants().ofType(uri) | %resource.descendants().ofType(url))) or descendants().where(reference = '#').exists()).not())",
    //     ];
    //
    //     for expr in exprs {
    //         let result = eval_r4_patient_expr(&patient, expr);
    //         println!("\nEXPR: {expr}\nRESULT: {result:#?}");
    //     }
    // }
    #[ignore]
    #[test]
    fn debug_patient_string_concat() {
        let left = EvaluationResult::string("hello".to_string());
        let right = EvaluationResult::string("world".to_string());

        let result =
            apply_additive(&left, "+", &right).expect("string concatenation should succeed");

        match result {
            EvaluationResult::String(value, ..) => assert_eq!(value, "helloworld"),
            other => panic!("expected string result, got {other:#?}"),
        }
    }
    #[ignore]
    #[test]
    fn debug_patient_hash_prefix_concat() {
        let left = EvaluationResult::string("#".to_string());
        let right = EvaluationResult::string("Org1".to_string());

        let result =
            apply_additive(&left, "+", &right).expect("string concatenation should succeed");

        match result {
            EvaluationResult::String(value, ..) => assert_eq!(value, "#Org1"),
            other => panic!("expected string result, got {other:#?}"),
        }
    }
}
