use helios_fhir::r4::{
    Bundle, Patient, ViewDefinition, ViewDefinitionSelect, ViewDefinitionSelectColumn,
};
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_extension_function_debug() {
    // Create test patient data with extensions (from SQL-on-FHIR test data)
    let patient_json = serde_json::json!({
        "resourceType": "Patient",
        "id": "pt1",
        "meta": {
            "profile": ["http://hl7.org/fhir/us/core/StructureDefinition/us-core-patient"]
        },
        "extension": [
            {
                "id": "birthsex",
                "url": "http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex",
                "valueCode": "F"
            },
            {
                "id": "race",
                "url": "http://hl7.org/fhir/us/core/StructureDefinition/us-core-race",
                "extension": [
                    {
                        "url": "ombCategory",
                        "valueCoding": {
                            "system": "urn:oid:2.16.840.1.113883.6.238",
                            "code": "2106-3",
                            "display": "White"
                        }
                    },
                    {
                        "url": "text",
                        "valueString": "Mixed"
                    }
                ]
            }
        ]
    });

    let patient: Patient = serde_json::from_value(patient_json).unwrap();

    // Create test bundle
    let bundle = Bundle {
        id: Some("test-bundle".to_string().into()),
        entry: Some(vec![helios_fhir::r4::BundleEntry {
            full_url: Some("urn:uuid:pt1".to_string().into()),
            resource: Some(helios_fhir::r4::Resource::Patient(patient)),
            ..Default::default()
        }]),
        ..Default::default()
    };

    // Create ViewDefinition to test extension function
    let view_def = ViewDefinition {
        id: Some("test-view".to_string().into()),
        resource: "Patient".to_string().into(),
        select: Some(vec![ViewDefinitionSelect {
            column: Some(vec![
                ViewDefinitionSelectColumn {
                    name: "id".to_string().into(),
                    path: "id".to_string().into(),
                    ..Default::default()
                },
                ViewDefinitionSelectColumn {
                    name: "birthsex".to_string().into(),
                    path: "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex').value.ofType(code).first()".to_string().into(),
                    ..Default::default()
                },
                ViewDefinitionSelectColumn {
                    name: "direct_extension_test".to_string().into(),
                    path: "extension('http://hl7.org/fhir/us/core/StructureDefinition/us-core-birthsex')".to_string().into(),
                    ..Default::default()
                }
            ]),
            ..Default::default()
        }]),
        ..Default::default()
    };

    // Execute the view definition
    let sof_view_def = SofViewDefinition::R4(view_def);
    let sof_bundle = SofBundle::R4(bundle);
    let result = run_view_definition(sof_view_def, sof_bundle, ContentType::Json);

    match result {
        Ok(json_output) => {
            let json_str = String::from_utf8(json_output).unwrap();
            println!("Extension test output: {}", json_str);

            // Parse the result to check values
            let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
            if let Some(rows) = parsed.as_array() {
                for row in rows {
                    println!("Row: {}", serde_json::to_string_pretty(row).unwrap());
                    if let Some(birthsex) = row.get("birthsex") {
                        println!("Birthsex value: {:?}", birthsex);
                        if !birthsex.is_null() {
                            println!("SUCCESS: Extension function returned non-null value");
                        } else {
                            println!("ISSUE: Extension function returned null");
                        }
                    }
                    if let Some(direct_ext) = row.get("direct_extension_test") {
                        println!("Direct extension test: {:?}", direct_ext);
                    }
                }
            }
        }
        Err(e) => {
            println!("Extension test failed: {:?}", e);
            panic!("Extension test failed: {:?}", e);
        }
    }
}
