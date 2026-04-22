mod tests {

    use fhir_validation::profile::cardinality::validate_min_cardinality;
    use fhir_validation::profile::types::ExtractedElementRule;
    use fhir_validation_types::{BindingDef, Severity};
    use serde_json::json;

    fn rule(path: &str, min: Option<u32>, max: Option<String>) -> ExtractedElementRule {
        ExtractedElementRule {
            id: path.to_string(),
            path: path.to_string(),
            min,
            max,
            binding: None::<BindingDef>,
            constraints: Vec::new(),
            value_constraint: None,
            type_constraints: vec![],
            slicing: None,
            slice_name: None,
            ..Default::default()
        }
    }

    #[test]
    fn missing_top_level_required_fields_produce_issues() {
        let patient = json!({
            "resourceType": "Patient"
        });

        let rules = vec![
            rule("Patient.identifier", Some(1), None),
            rule("Patient.gender", Some(1), None),
            rule("Patient.birthDate", Some(1), None),
        ];

        let issues = validate_min_cardinality(&patient, "Patient", &rules);

        assert_eq!(issues.len(), 3);
        assert!(issues.iter().all(|i| i.severity == Severity::Error));
        assert!(issues.iter().any(|i| i.fhir_path == "Patient.identifier"));
        assert!(issues.iter().any(|i| i.fhir_path == "Patient.gender"));
        assert!(issues.iter().any(|i| i.fhir_path == "Patient.birthDate"));
    }

    #[test]
    fn present_scalar_and_array_values_satisfy_minimum_cardinality() {
        let patient = json!({
            "resourceType": "Patient",
            "identifier": [{ "system": "http://atrius.health/mrn", "value": "12345" }],
            "gender": "male",
            "birthDate": "1980-01-01"
        });

        let rules = vec![
            rule("Patient.identifier", Some(1), None),
            rule("Patient.gender", Some(1), None),
            rule("Patient.birthDate", Some(1), None),
        ];

        let issues = validate_min_cardinality(&patient, "Patient", &rules);
        assert!(issues.is_empty());
    }

    #[test]
    fn zero_minimum_is_ignored() {
        let patient = json!({
            "resourceType": "Patient"
        });

        let rules = vec![rule("Patient.maritalStatus", Some(0), None)];
        let issues = validate_min_cardinality(&patient, "Patient", &rules);

        assert!(issues.is_empty());
    }

    #[test]
    fn nested_paths_are_counted() {
        let patient = json!({
            "resourceType": "Patient",
            "contact": {
                "name": {
                    "text": "Jane Doe"
                }
            }
        });

        let rules = vec![rule("Patient.contact.name", Some(1), None)];
        let issues = validate_min_cardinality(&patient, "Patient", &rules);

        assert!(issues.is_empty());
    }
}
