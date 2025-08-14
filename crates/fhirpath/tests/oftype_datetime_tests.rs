use helios_fhirpath::{EvaluationContext, evaluate_expression};

#[test]
fn test_oftype_datetime_conversion() {
    let context = EvaluationContext::new_empty_with_default_version();

    // Test if ofType(dateTime) converts a Date to DateTime
    println!(
        "Date: {:?}",
        evaluate_expression("@2010-10-10", &context).unwrap()
    );
    println!(
        "Date.ofType(dateTime): {:?}",
        evaluate_expression("@2010-10-10.ofType(dateTime)", &context).unwrap()
    );
    println!(
        "Date.ofType(System.DateTime): {:?}",
        evaluate_expression("@2010-10-10.ofType(System.DateTime)", &context).unwrap()
    );

    // Test DateTime boundaries after ofType conversion
    println!(
        "Date.ofType(dateTime).lowBoundary(): {:?}",
        evaluate_expression("@2010-10-10.ofType(dateTime).lowBoundary()", &context).unwrap()
    );
}
