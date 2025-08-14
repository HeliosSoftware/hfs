#[cfg(test)]
mod tests {
    use helios_fhir::{FhirResource, r4::Patient};
    use helios_fhirpath::{EvaluationContext, evaluate_expression};

    #[test]
    fn debug_boolean_constant_test() {
        // Create a test patient with deceasedBoolean = true
        let patient_json = r#"
        {
            "resourceType": "Patient",
            "id": "pt2",
            "deceasedBoolean": true,
            "name": [{
                "family": "Johnson",
                "use": "usual"
            }]
        }
        "#;

        let patient: Patient = serde_json::from_str(patient_json).unwrap();
        println!("Patient: {:#?}", patient);

        // Check what the deceased field contains
        if let Some(deceased) = &patient.deceased {
            println!("Deceased: {:#?}", deceased);
        } else {
            println!("No deceased field");
        }

        // Convert patient to FhirResource and EvaluationContext
        let fhir_resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(patient)));
        let mut context = EvaluationContext::new(vec![fhir_resource]);

        // Add the boolean constant
        let is_deceased_constant = helios_fhirpath_support::EvaluationResult::Boolean(
            true,
            Some(helios_fhirpath_support::TypeInfoResult::new(
                "FHIR", "boolean",
            )),
        );
        context.set_variable_result("is_deceased", is_deceased_constant);

        // Test the individual parts of the expression
        println!("\n=== Testing FHIRPath expressions ===");

        // Test: deceased
        match evaluate_expression("deceased", &context) {
            Ok(result) => println!("deceased = {:#?}", result),
            Err(e) => println!("deceased error: {:?}", e),
        }

        // Test: deceased.ofType(boolean)
        match evaluate_expression("deceased.ofType(boolean)", &context) {
            Ok(result) => println!("deceased.ofType(boolean) = {:#?}", result),
            Err(e) => println!("deceased.ofType(boolean) error: {:?}", e),
        }

        // Test: deceased.ofType(boolean).exists()
        match evaluate_expression("deceased.ofType(boolean).exists()", &context) {
            Ok(result) => println!("deceased.ofType(boolean).exists() = {:#?}", result),
            Err(e) => println!("deceased.ofType(boolean).exists() error: {:?}", e),
        }

        // Test: %is_deceased
        match evaluate_expression("%is_deceased", &context) {
            Ok(result) => println!("%is_deceased = {:#?}", result),
            Err(e) => println!("%is_deceased error: {:?}", e),
        }

        // Test: deceased.ofType(boolean) = %is_deceased
        match evaluate_expression("deceased.ofType(boolean) = %is_deceased", &context) {
            Ok(result) => println!("deceased.ofType(boolean) = %is_deceased = {:#?}", result),
            Err(e) => println!("deceased.ofType(boolean) = %is_deceased error: {:?}", e),
        }

        // Test: the full expression
        match evaluate_expression(
            "deceased.ofType(boolean).exists() and deceased.ofType(boolean) = %is_deceased",
            &context,
        ) {
            Ok(result) => println!("Full expression = {:#?}", result),
            Err(e) => println!("Full expression error: {:?}", e),
        }

        // This test always passes - we just want to see the debug output
    }
}
