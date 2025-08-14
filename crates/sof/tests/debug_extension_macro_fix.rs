use helios_fhirpath::{EvaluationContext, evaluate_expression};

#[test]
fn debug_extension_macro_fix() {
    // Create test data matching the SQL-on-FHIR extension test case
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "pt1",
        "extension": [
            {
                "id": "birthsex",
                "url": "http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex",
                "valueCode": "F"
            }
        ]
    });

    // Parse into FHIR resource
    let patient: helios_fhir::r4::Patient =
        serde_json::from_value(patient_json).expect("Failed to parse patient");

    println!("Patient parsed successfully");

    // Create context with the patient
    let context = EvaluationContext::new(vec![helios_fhir::FhirResource::R4(Box::new(
        helios_fhir::r4::Resource::Patient(patient),
    ))]);

    println!("Context created");

    // Test the full extension expression that's failing in SQL-on-FHIR
    println!("\n=== Testing full extension expression ===");
    test_expression(
        "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code).first()",
        &context,
    );

    // Test step by step to see where it breaks
    println!("\n=== Step by step debugging ===");
    test_expression("extension", &context);
    test_expression(
        "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex')",
        &context,
    );
    test_expression(
        "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value",
        &context,
    );
    test_expression(
        "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').valueCode",
        &context,
    );
    test_expression(
        "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code)",
        &context,
    );
    test_expression(
        "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(String)",
        &context,
    );

    println!("\n=== Testing alternative approaches ===");
    test_expression(
        "extension.where(url = 'http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').valueCode",
        &context,
    );
    test_expression(
        "extension.where(url = 'http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value",
        &context,
    );
}

fn test_expression(expr_str: &str, context: &EvaluationContext) {
    println!("\nTesting: {}", expr_str);

    match evaluate_expression(expr_str, context) {
        Ok(result) => {
            println!("  Result: {:?}", result);
        }
        Err(e) => {
            println!("  Error: {}", e);
        }
    }
}
