#[test]
fn test_raw_fhir_parsing() {
    // Test the raw FHIR parsing to see exactly what structure we get
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

    println!("Original JSON:");
    println!("{}", serde_json::to_string_pretty(&patient_json).unwrap());

    // Parse as R4 Patient
    let patient: helios_fhir::r4::Patient =
        serde_json::from_value(patient_json.clone()).expect("Failed to parse patient");

    // Serialize back to JSON to see what structure we get
    let serialized = serde_json::to_value(&patient).expect("Failed to serialize patient");
    println!("\nAfter R4 Patient parsing and serialization:");
    println!("{}", serde_json::to_string_pretty(&serialized).unwrap());

    // Check the extension structure specifically
    if let Some(extensions) = serialized.get("extension") {
        if let Some(first_ext) = extensions.as_array().and_then(|arr| arr.first()) {
            println!("\nFirst extension structure:");
            println!("{}", serde_json::to_string_pretty(first_ext).unwrap());

            // Check for valueCode vs value
            if first_ext.get("valueCode").is_some() {
                println!("Extension has 'valueCode' field");
            }
            if first_ext.get("value").is_some() {
                println!("Extension has 'value' field");
            }
        }
    }

    // Test the extension field directly
    if let Some(extensions) = &patient.extension {
        if let Some(first_ext) = extensions.first() {
            println!("\nDirect extension access:");
            println!("Extension ID: {:?}", first_ext.id);
            println!("Extension URL: {:?}", first_ext.url);

            // Check the value field - this might be a choice element
            // Let's see what type it actually is
            if let Some(value) = &first_ext.value {
                println!("Extension value field exists");
                // Serialize just the value to see its structure
                let value_json = serde_json::to_value(value).expect("Failed to serialize value");
                println!(
                    "Value structure: {}",
                    serde_json::to_string_pretty(&value_json).unwrap()
                );
            } else {
                println!("Extension value field is None");
            }
        }
    }
}
