use helios_fhirpath_support::IntoEvaluationResult;

#[test]
fn test_evaluation_result_conversion() {
    // Create a patient with extension
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

    // Parse as R4 Patient
    let patient: helios_fhir::r4::Patient =
        serde_json::from_value(patient_json).expect("Failed to parse patient");

    // Convert to EvaluationResult
    let eval_result = patient.to_evaluation_result();

    println!("EvaluationResult structure:");
    println!("{:#?}", eval_result);

    // Test individual extension conversion
    if let Some(extensions) = &patient.extension {
        if let Some(first_ext) = extensions.first() {
            println!("\nDirect extension conversion:");
            let ext_eval_result = first_ext.to_evaluation_result();
            println!("{:#?}", ext_eval_result);

            // Test the value field specifically
            if let Some(value) = &first_ext.value {
                println!("\nExtension value conversion:");
                let value_eval_result = value.to_evaluation_result();
                println!("{:#?}", value_eval_result);
            }
        }
    }
}
