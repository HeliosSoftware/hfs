use helios_fhirpath::{EvaluationContext, evaluate_expression};

#[test]
fn debug_reference_key_functions() {
    // Create test data matching the reference key test
    let patient_p1_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p1",
        "link": [
            {
                "other": {
                    "reference": "Patient/p1"
                }
            }
        ]
    });

    let patient_p2_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p2",
        "link": [
            {
                "other": {
                    "reference": "Patient/p3"
                }
            }
        ]
    });

    // Parse into FHIR resources
    let patient_p1: helios_fhir::r4::Patient = serde_json::from_value(patient_p1_json).unwrap();
    let patient_p2: helios_fhir::r4::Patient = serde_json::from_value(patient_p2_json).unwrap();

    println!("=== Testing Reference Key Functions ===\n");

    // Test each patient separately
    for (name, patient) in [("p1", patient_p1), ("p2", patient_p2)] {
        println!("--- Patient {} ---", name);

        let context = EvaluationContext::new(vec![helios_fhir::FhirResource::R4(Box::new(
            helios_fhir::r4::Resource::Patient(Box::new(patient)),
        ))]);

        // Test individual components
        println!(
            "getResourceKey(): {:?}",
            evaluate_expression("getResourceKey()", &context).unwrap()
        );
        println!(
            "link.other: {:?}",
            evaluate_expression("link.other", &context).unwrap()
        );
        println!(
            "link.other.reference: {:?}",
            evaluate_expression("link.other.reference", &context).unwrap()
        );
        println!(
            "link.other.getReferenceKey(): {:?}",
            evaluate_expression("link.other.getReferenceKey()", &context).unwrap()
        );
        println!(
            "link.other.getReferenceKey(Patient): {:?}",
            evaluate_expression("link.other.getReferenceKey(Patient)", &context).unwrap()
        );
        println!(
            "link.other.getReferenceKey('Patient'): {:?}",
            evaluate_expression("link.other.getReferenceKey('Patient')", &context).unwrap()
        );

        // Test what Patient evaluates to
        println!(
            "Patient identifier: {:?}",
            evaluate_expression("Patient", &context).unwrap()
        );

        // Test the full expressions
        println!(
            "Without type filter: {:?}",
            evaluate_expression("getResourceKey() = link.other.getReferenceKey()", &context)
                .unwrap()
        );
        println!(
            "With type filter: {:?}",
            evaluate_expression(
                "getResourceKey() = link.other.getReferenceKey(Patient)",
                &context
            )
            .unwrap()
        );

        println!();
    }
}
