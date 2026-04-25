use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, evaluate_expression};

#[test]
fn debug_datetime_boundary_issue() {
    // Create an observation with a dateTime value exactly like the SQL-on-FHIR test
    let observation_json = serde_json::json!({
        "resourceType": "Observation",
        "id": "o2",
        "code": {
            "text": "code"
        },
        "status": "final",
        "valueDateTime": "2010-10-10"
    });

    let observation: helios_fhir::r4::Observation =
        serde_json::from_value(observation_json).unwrap();
    let resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Observation(Box::new(
        observation,
    ))));
    let context = EvaluationContext::new(vec![resource]);

    // Debug the step-by-step evaluation
    println!("=== Debugging datetime boundary issue ===");

    // Check what's in the observation
    println!(
        "1. Raw valueDateTime: {:?}",
        evaluate_expression("valueDateTime", &context).unwrap()
    );

    // Check polymorphic value access
    println!(
        "2. Polymorphic value: {:?}",
        evaluate_expression("value", &context).unwrap()
    );

    // Check what valueDateTime field contains
    println!(
        "2a. Checking valueDateTime field: {:?}",
        evaluate_expression("valueDateTime", &context).unwrap()
    );

    // Check basic fields on the observation
    println!(
        "2b. Status field: {:?}",
        evaluate_expression("status", &context).unwrap()
    );

    // Check the id field to make sure basic access works
    println!(
        "2c. ID field: {:?}",
        evaluate_expression("id", &context).unwrap()
    );

    // Check the type casting
    println!(
        "3. value.ofType(dateTime): {:?}",
        evaluate_expression("value.ofType(dateTime)", &context).unwrap()
    );

    // Check what happens with a literal datetime
    println!(
        "4. Literal datetime boundary: {:?}",
        evaluate_expression("@2010-10-10.lowBoundary()", &context).unwrap()
    );

    // Check if the issue is with ofType or lowBoundary
    println!(
        "5. valueDateTime.lowBoundary(): {:?}",
        evaluate_expression("valueDateTime.lowBoundary()", &context).unwrap()
    );

    // Try direct datetime boundary
    println!(
        "6. @2010-10-10T00:00:00.lowBoundary(): {:?}",
        evaluate_expression("@2010-10-10T00:00:00.lowBoundary()", &context).unwrap()
    );

    // Final test - the exact failing expression
    println!(
        "7. value.ofType(dateTime).lowBoundary(): {:?}",
        evaluate_expression("value.ofType(dateTime).lowBoundary()", &context).unwrap()
    );
}

#[test]
fn debug_datetime_type_conversion() {
    let context = EvaluationContext::new_empty_with_default_version();

    println!("=== Testing datetime type conversion ===");

    // Test various date formats and their boundary functions
    let date_formats = vec![
        "@2010-10-10",
        "@2010-10-10T12:30",
        "@2010-10-10T12:30:45",
        "@2010-10-10T12:30:45.123",
    ];

    for date_format in date_formats {
        println!("\nTesting: {}", date_format);
        println!(
            "  lowBoundary: {:?}",
            evaluate_expression(&format!("{}.lowBoundary()", date_format), &context).unwrap()
        );
        println!(
            "  highBoundary: {:?}",
            evaluate_expression(&format!("{}.highBoundary()", date_format), &context).unwrap()
        );
    }
}
