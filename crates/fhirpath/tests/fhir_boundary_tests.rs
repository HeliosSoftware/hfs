use helios_fhir::FhirResource;
use helios_fhirpath::{EvaluationContext, evaluate_expression};

#[test]
fn test_fhir_observation_boundary() {
    // Create an observation with a decimal value
    let observation_json = serde_json::json!({
        "resourceType": "Observation",
        "id": "o1",
        "code": {
            "text": "code"
        },
        "status": "final",
        "valueQuantity": {
            "value": 1.0
        }
    });

    let observation: helios_fhir::r4::Observation =
        serde_json::from_value(observation_json).unwrap();
    let resource = FhirResource::R4(Box::new(helios_fhir::r4::Resource::Observation(Box::new(
        observation,
    ))));
    let context = EvaluationContext::new(vec![resource]);

    // Test the boundary function on Quantity value
    println!(
        "Observation value: {:?}",
        evaluate_expression("value", &context).unwrap()
    );
    println!(
        "Observation valueQuantity: {:?}",
        evaluate_expression("valueQuantity", &context).unwrap()
    );
    println!(
        "Observation valueQuantity.value: {:?}",
        evaluate_expression("valueQuantity.value", &context).unwrap()
    );
    println!(
        "Observation value.ofType(Quantity): {:?}",
        evaluate_expression("value.ofType(Quantity)", &context).unwrap()
    );
    println!(
        "Observation value.ofType(Quantity).value: {:?}",
        evaluate_expression("value.ofType(Quantity).value", &context).unwrap()
    );
    println!(
        "Observation value.ofType(Quantity).value.lowBoundary(): {:?}",
        evaluate_expression("value.ofType(Quantity).value.lowBoundary()", &context).unwrap()
    );
}

#[test]
fn test_fhir_datetime_observation_boundary() {
    // Create an observation with a dateTime value
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

    // Test the boundary function on DateTime value
    println!(
        "Observation valueDateTime: {:?}",
        evaluate_expression("valueDateTime", &context).unwrap()
    );
    println!(
        "Observation value: {:?}",
        evaluate_expression("value", &context).unwrap()
    );
    println!(
        "Observation value.ofType(dateTime): {:?}",
        evaluate_expression("value.ofType(dateTime)", &context).unwrap()
    );
    println!(
        "Observation value.ofType(dateTime).lowBoundary(): {:?}",
        evaluate_expression("value.ofType(dateTime).lowBoundary()", &context).unwrap()
    );
}

#[test]
fn test_fhir_patient_boundary() {
    // Create a patient with a birthDate
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

    // Test the boundary function on birthDate
    println!(
        "Patient birthDate: {:?}",
        evaluate_expression("birthDate", &context).unwrap()
    );
    println!(
        "Patient birthDate.lowBoundary(): {:?}",
        evaluate_expression("birthDate.lowBoundary()", &context).unwrap()
    );
    println!(
        "Patient birthDate.highBoundary(): {:?}",
        evaluate_expression("birthDate.highBoundary()", &context).unwrap()
    );
}
