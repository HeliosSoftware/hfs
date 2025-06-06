use fhirpath::{evaluate_expression, EvaluationContext};
use serde_json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let patient_p1: fhir::r4::Patient = serde_json::from_value(patient_p1_json)?;
    let patient_p2: fhir::r4::Patient = serde_json::from_value(patient_p2_json)?;

    println!("=== Testing Reference Key Functions ===\n");

    // Test each patient separately
    for (name, patient) in [("p1", patient_p1), ("p2", patient_p2)] {
        println!("--- Patient {} ---", name);
        
        let context = EvaluationContext::new(vec![
            fhir::FhirResource::R4(Box::new(fhir::r4::Resource::Patient(patient)))
        ]);

        // Test individual components
        println!("getResourceKey(): {:?}", evaluate_expression("getResourceKey()", &context)?);
        println!("link.other: {:?}", evaluate_expression("link.other", &context)?);
        println!("link.other.reference: {:?}", evaluate_expression("link.other.reference", &context)?);
        println!("link.other.getReferenceKey(): {:?}", evaluate_expression("link.other.getReferenceKey()", &context)?);
        println!("link.other.getReferenceKey(Patient): {:?}", evaluate_expression("link.other.getReferenceKey(Patient)", &context)?);
        
        // Test the full expressions
        println!("Without type filter: {:?}", evaluate_expression("getResourceKey() = link.other.getReferenceKey()", &context)?);
        println!("With type filter: {:?}", evaluate_expression("getResourceKey() = link.other.getReferenceKey(Patient)", &context)?);
        
        println!();
    }

    Ok(())
}