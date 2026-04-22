mod tests {
    use crate::common::fixtures::{assert_has_invariant, assert_issue_count, assert_no_errors};
    use crate::common::fixtures::{assert_has_warning, load_resource, local_terminology_r4};
    use fhir_validation::R4FhirPathEvaluator;
    use helios_fhir::{FhirResource, FhirVersion};

    pub fn r4_evaluator_for(resource: &FhirResource) -> R4FhirPathEvaluator {
        let FhirResource::R4(r) = resource else {
            panic!("expected R4 resource");
        };
        R4FhirPathEvaluator::new((**r).clone())
    }
    #[test]
    fn local_reference_with_matching_contained_resource_is_valid() {
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
    // Next reference coverage to enable after adding the corresponding fixtures.
    #[ignore]
    #[test]
    fn patient_absent_optional_reference_passes() {
        let r = load_resource(
            FhirVersion::R4,
            "valid/patient/patient_without_managing_organization.json",
        );
        let validator = fhir_validation::Validator::default();
        let evaluator = r4_evaluator_for(&r);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&r, Some(&term), &evaluator);

        assert_no_errors(&issues);
    }
    #[ignore]
    #[test]
    fn patient_local_reference_with_multiple_contained_resources_match_is_valid() {
        let r = load_resource(
            FhirVersion::R4,
            "valid/patient/patient_multiple_contained_match.json",
        );
        let validator = fhir_validation::Validator::default();
        let evaluator = r4_evaluator_for(&r);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&r, Some(&term), &evaluator);

        assert_no_errors(&issues);
    }
    #[ignore]
    #[test]
    fn patient_local_reference_with_multiple_contained_resources_no_match_emits_invariant() {
        let r = load_resource(
            FhirVersion::R4,
            "invalid/patient/patient_multiple_contained_no_match.json",
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
        assert_has_invariant(
            &issues,
            "Patient",
            "If the resource is contained in another resource, it SHALL be referred to from elsewhere in the resource or SHALL refer to the containing resource",
        );
    }
    #[test]
    fn contained_resource_referencing_parent_is_valid() {
        let r = load_resource(
            FhirVersion::R4,
            "valid/patient/patient_contained_references_parent.json",
        );

        let validator = fhir_validation::Validator::default();
        let evaluator = r4_evaluator_for(&r);
        let term = local_terminology_r4();
        let issues = validator.validate_resource(&r, Some(&term), &evaluator);

        assert_no_errors(&issues);
        assert_has_warning(&issues);
    }
}
