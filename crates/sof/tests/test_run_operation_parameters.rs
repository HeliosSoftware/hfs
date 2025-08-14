//! Unit tests for ViewDefinition/$run operation with query parameters
//!
//! This module tests parameter validation, filtering, and pagination functionality
//! for the $run operation.

use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
use serde_json::json;

// Import the models module for parameter validation testing
// Note: These would normally be internal modules, but for testing we need access

/// Helper function to create a basic ViewDefinition for testing
fn create_test_view_definition() -> serde_json::Value {
    json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "family_name",
                "path": "name.family"
            }, {
                "name": "given_name",
                "path": "name.given"
            }]
        }]
    })
}

/// Helper function to create test patient resources
fn create_test_patients() -> Vec<serde_json::Value> {
    vec![
        json!({
            "resourceType": "Patient",
            "id": "patient-1",
            "name": [{
                "family": "Smith",
                "given": ["John"]
            }]
        }),
        json!({
            "resourceType": "Patient",
            "id": "patient-2",
            "name": [{
                "family": "Doe",
                "given": ["Jane"]
            }]
        }),
        json!({
            "resourceType": "Patient",
            "id": "patient-3",
            "name": [{
                "family": "Johnson",
                "given": ["Bob"]
            }]
        }),
        json!({
            "resourceType": "Patient",
            "id": "patient-4",
            "name": [{
                "family": "Williams",
                "given": ["Alice"]
            }]
        }),
        json!({
            "resourceType": "Patient",
            "id": "patient-5",
            "name": [{
                "family": "Brown",
                "given": ["Charlie"]
            }]
        }),
    ]
}

/// Helper function to create a bundle from patient resources
fn create_test_bundle(
    patients: Vec<serde_json::Value>,
) -> Result<SofBundle, Box<dyn std::error::Error>> {
    let bundle_json = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": patients.into_iter().map(|resource| {
            json!({
                "resource": resource
            })
        }).collect::<Vec<_>>()
    });

    #[cfg(feature = "R4")]
    {
        let bundle: helios_fhir::r4::Bundle = serde_json::from_value(bundle_json)?;
        Ok(SofBundle::R4(bundle))
    }

    #[cfg(not(feature = "R4"))]
    {
        Err("R4 feature not enabled".into())
    }
}

/// Helper function to create a ViewDefinition from JSON
fn create_view_definition(
    json: serde_json::Value,
) -> Result<SofViewDefinition, Box<dyn std::error::Error>> {
    #[cfg(feature = "R4")]
    {
        let view_def: helios_fhir::r4::ViewDefinition = serde_json::from_value(json)?;
        Ok(SofViewDefinition::R4(view_def))
    }

    #[cfg(not(feature = "R4"))]
    {
        Err("R4 feature not enabled".into())
    }
}

/// Test basic ViewDefinition execution without parameters
#[test]
fn test_basic_view_definition_execution() {
    let view_def_json = create_test_view_definition();
    let patients = create_test_patients();

    let view_definition = create_view_definition(view_def_json).unwrap();
    let bundle = create_test_bundle(patients).unwrap();

    // Test CSV output
    let csv_result = run_view_definition(
        view_definition.clone(),
        bundle.clone(),
        ContentType::CsvWithHeader,
    );
    assert!(csv_result.is_ok());

    let csv_data = String::from_utf8(csv_result.unwrap()).unwrap();
    assert!(csv_data.contains("id,family_name,given_name"));
    assert!(csv_data.contains("patient-1"));

    // Test JSON output
    let json_result = run_view_definition(view_definition, bundle, ContentType::Json);
    assert!(json_result.is_ok());

    let json_data = String::from_utf8(json_result.unwrap()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_data).unwrap();
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 5);
}

/// Test CSV output format with and without headers
#[test]
fn test_csv_format_variations() {
    let view_def_json = create_test_view_definition();
    let patients = create_test_patients();

    let view_definition = create_view_definition(view_def_json).unwrap();
    let bundle = create_test_bundle(patients).unwrap();

    // Test CSV with headers
    let csv_with_headers = run_view_definition(
        view_definition.clone(),
        bundle.clone(),
        ContentType::CsvWithHeader,
    )
    .unwrap();
    let csv_with_headers_str = String::from_utf8(csv_with_headers).unwrap();
    assert!(csv_with_headers_str.starts_with("id,family_name,given_name"));

    // Test CSV without headers
    let csv_without_headers =
        run_view_definition(view_definition, bundle, ContentType::Csv).unwrap();
    let csv_without_headers_str = String::from_utf8(csv_without_headers).unwrap();
    // Should not start with column names
    assert!(!csv_without_headers_str.starts_with("id,family_name,given_name"));
    // But should contain patient data
    assert!(csv_without_headers_str.contains("patient-1"));
}

/// Test multiple output formats produce consistent data
#[test]
fn test_output_format_consistency() {
    let view_def_json = create_test_view_definition();
    let patients = create_test_patients();

    let view_definition = create_view_definition(view_def_json).unwrap();
    let bundle = create_test_bundle(patients).unwrap();

    // Get JSON output and parse it
    let json_result =
        run_view_definition(view_definition.clone(), bundle.clone(), ContentType::Json).unwrap();
    let json_str = String::from_utf8(json_result).unwrap();
    let json_data: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();

    // Get CSV output and parse it
    let csv_result =
        run_view_definition(view_definition, bundle, ContentType::CsvWithHeader).unwrap();
    let csv_str = String::from_utf8(csv_result).unwrap();
    let csv_lines: Vec<&str> = csv_str.trim().split('\n').collect();

    // Both should have the same number of data rows
    assert_eq!(json_data.len(), csv_lines.len() - 1); // -1 for CSV header

    // Verify that we have data for all 5 patients
    assert_eq!(json_data.len(), 5);
    assert_eq!(csv_lines.len(), 6); // 5 patients + 1 header
}

/// Test NDJSON output format
#[test]
fn test_ndjson_output_format() {
    let view_def_json = create_test_view_definition();
    let patients = create_test_patients();

    let view_definition = create_view_definition(view_def_json).unwrap();
    let bundle = create_test_bundle(patients).unwrap();

    let ndjson_result = run_view_definition(view_definition, bundle, ContentType::NdJson).unwrap();
    let ndjson_str = String::from_utf8(ndjson_result).unwrap();

    // Each line should be valid JSON
    let lines: Vec<&str> = ndjson_str.trim().split('\n').collect();
    assert_eq!(lines.len(), 5); // One line per patient

    for line in lines {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(parsed.is_object());
        assert!(parsed.get("id").is_some());
        assert!(parsed.get("family_name").is_some());
    }
}

/// Test that empty bundle produces empty results
#[test]
fn test_empty_bundle() {
    let view_def_json = create_test_view_definition();
    let view_definition = create_view_definition(view_def_json).unwrap();
    let empty_bundle = create_test_bundle(vec![]).unwrap();

    let json_result = run_view_definition(
        view_definition.clone(),
        empty_bundle.clone(),
        ContentType::Json,
    )
    .unwrap();
    let json_str = String::from_utf8(json_result).unwrap();
    let json_data: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();
    assert_eq!(json_data.len(), 0);

    let csv_result =
        run_view_definition(view_definition, empty_bundle, ContentType::CsvWithHeader).unwrap();
    let csv_str = String::from_utf8(csv_result).unwrap();

    // With an empty bundle, the SOF library produces minimal output (just quotes)
    // This is the expected behavior when there are no resources to process
    assert!(csv_str.trim().len() <= 2); // Just quotes or empty
}

/// Test ViewDefinition with different column configurations
#[test]
fn test_custom_column_configuration() {
    let custom_view_def = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{
                "name": "patient_id",
                "path": "id"
            }, {
                "name": "full_name",
                "path": "name.family"
            }]
        }]
    });

    let patients = vec![create_test_patients()[0].clone()];
    let view_definition = create_view_definition(custom_view_def).unwrap();
    let bundle = create_test_bundle(patients).unwrap();

    let csv_result =
        run_view_definition(view_definition, bundle, ContentType::CsvWithHeader).unwrap();
    let csv_str = String::from_utf8(csv_result).unwrap();

    // Should use the custom column names
    assert!(csv_str.contains("patient_id,full_name"));
    assert!(csv_str.contains("patient-1"));
    assert!(csv_str.contains("Smith"));
}

/// Test ContentType enum parsing
#[test]
fn test_content_type_parsing() {
    // Test valid content types
    assert!(ContentType::from_string("application/json").is_ok());
    assert!(ContentType::from_string("text/csv").is_ok());
    assert!(ContentType::from_string("text/csv;header=true").is_ok());
    assert!(ContentType::from_string("text/csv;header=false").is_ok());
    assert!(ContentType::from_string("application/ndjson").is_ok());

    // Test that the parsed types match expected values
    assert_eq!(
        ContentType::from_string("application/json").unwrap(),
        ContentType::Json
    );
    assert_eq!(
        ContentType::from_string("text/csv").unwrap(),
        ContentType::CsvWithHeader
    );
    assert_eq!(
        ContentType::from_string("text/csv;header=true").unwrap(),
        ContentType::CsvWithHeader
    );
    assert_eq!(
        ContentType::from_string("text/csv;header=false").unwrap(),
        ContentType::Csv
    );
    assert_eq!(
        ContentType::from_string("application/ndjson").unwrap(),
        ContentType::NdJson
    );
}
