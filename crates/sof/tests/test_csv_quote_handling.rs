use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
use serde_json::json;

#[cfg(feature = "R4")]
#[tokio::test]
async fn test_getresourcekey_csv_quote_handling() {
    // Create ViewDefinition with getResourceKey() function
    let view_definition = json!({
        "resourceType": "ViewDefinition",
        "id": "PatientDemographics",
        "name": "patient_demographics",
        "status": "draft",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {
                        "path": "getResourceKey()",
                        "name": "id"
                    }
                ]
            }
        ]
    });

    // Create Bundle with test patients
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "269a6a02-37e2-bd1a-e079-fda944434a99",
                    "name": [{
                        "use": "official",
                        "family": "Cole",
                        "given": ["Joanie"]
                    }],
                    "birthDate": "2012-03-30"
                }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "269a6a02-37e2-bd1a-e079-fda944434a98",
                    "name": [{
                        "use": "official",
                        "family": "Munini",
                        "given": ["Steve"]
                    }],
                    "birthDate": "1972-03-30"
                }
            }
        ]
    });

    // Parse as version-specific types and convert to SofViewDefinition/SofBundle
    // Using R4 feature since that's what's enabled by default
    let view_def_r4: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(view_definition).unwrap();
    let bundle_r4: helios_fhir::r4::Bundle = serde_json::from_value(bundle).unwrap();

    let view_def = SofViewDefinition::R4(view_def_r4);
    let bundle = SofBundle::R4(bundle_r4);

    // Run the view definition with CSV output (with headers)
    let result = run_view_definition(view_def, bundle, ContentType::CsvWithHeader).unwrap();
    let csv_output = std::str::from_utf8(&result).unwrap();

    // Split into lines to check each row
    let lines: Vec<&str> = csv_output.trim().split('\n').collect();

    // Check header
    assert_eq!(lines[0], "id");

    // Check that resource keys don't have extra quotes
    // They should be: 269a6a02-37e2-bd1a-e079-fda944434a99
    // Not: """269a6a02-37e2-bd1a-e079-fda944434a99"""
    assert_eq!(lines[1], "269a6a02-37e2-bd1a-e079-fda944434a99");
    assert_eq!(lines[2], "269a6a02-37e2-bd1a-e079-fda944434a98");

    // Ensure we're not getting triple quotes
    assert!(!lines[1].contains("\"\"\""));
    assert!(!lines[2].contains("\"\"\""));
}
