use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, evaluate_expression};

#[test]
fn test_string_boundary_functions() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test with direct strings
    println!("Test date string boundary:");
    let result = evaluate_expression("'1970-06'.lowBoundary()", &context).unwrap();
    println!("'1970-06'.lowBoundary(): {:?}", result);

    let result = evaluate_expression("'1970-06'.highBoundary()", &context).unwrap();
    println!("'1970-06'.highBoundary(): {:?}", result);

    println!("\nTest datetime string boundary:");
    let result = evaluate_expression("'2010-10-10'.lowBoundary()", &context).unwrap();
    println!("'2010-10-10'.lowBoundary(): {:?}", result);

    let result = evaluate_expression("'2010-10-10'.highBoundary()", &context).unwrap();
    println!("'2010-10-10'.highBoundary(): {:?}", result);

    println!("\nTest time string boundary:");
    let result = evaluate_expression("'12:34'.lowBoundary()", &context).unwrap();
    println!("'12:34'.lowBoundary(): {:?}", result);

    let result = evaluate_expression("'12:34'.highBoundary()", &context).unwrap();
    println!("'12:34'.highBoundary(): {:?}", result);
}

#[test]
fn test_fhir_types_detailed() {
    // Create a patient with multiple date types
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "p1",
        "birthDate": "1970-06"
    });

    let patient: helios_fhir::r4::Patient = serde_json::from_value(patient_json).unwrap();
    let resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Patient(Box::new(
        patient,
    ))));
    let context = EvaluationContext::new(vec![resource]);

    // Test type operations
    println!(
        "Patient.birthDate: {:?}",
        evaluate_expression("birthDate", &context).unwrap()
    );
    println!(
        "Patient.birthDate.type(): {:?}",
        evaluate_expression("birthDate.type()", &context).unwrap()
    );

    // Test boundary after explicit string handling
    println!(
        "Patient.birthDate.lowBoundary(): {:?}",
        evaluate_expression("birthDate.lowBoundary()", &context).unwrap()
    );
}
