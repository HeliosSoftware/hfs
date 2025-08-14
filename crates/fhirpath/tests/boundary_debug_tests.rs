use helios_fhirpath::{EvaluationContext, evaluate_expression};

#[test]
fn test_debug_boundary_inputs() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Check what types we get for different literals
    println!(
        "Date literal: {:?}",
        evaluate_expression("@2010-10-10", &context).unwrap()
    );
    println!(
        "DateTime literal: {:?}",
        evaluate_expression("@2010-10-10T00:00:00", &context).unwrap()
    );
    println!(
        "Time literal: {:?}",
        evaluate_expression("@T12:34", &context).unwrap()
    );

    // Test boundary functions on these types
    println!(
        "Date boundary: {:?}",
        evaluate_expression("@2010-10-10.lowBoundary()", &context).unwrap()
    );
    println!(
        "DateTime boundary: {:?}",
        evaluate_expression("@2010-10-10T00:00:00.lowBoundary()", &context).unwrap()
    );
    println!(
        "Time boundary: {:?}",
        evaluate_expression("@T12:34.lowBoundary()", &context).unwrap()
    );
}
