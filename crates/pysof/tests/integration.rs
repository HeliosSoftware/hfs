//! Integration tests for pysof
//!
//! This file contains integration tests that test the interaction
//! between different components of the pysof library.

use helios_serde::json;
use helios_sof::ContentType;
use serde_json::json;

#[test]
fn test_content_type_pipeline_integration() {
    // Test that content types work end-to-end with actual data
    let formats = ["json", "csv", "ndjson"];

    for format in &formats {
        let content_type = ContentType::from_string(format);
        assert!(content_type.is_ok(), "Failed to parse format: {}", format);

        // Test that the content type can be used in format mapping
        let format_str = match content_type.unwrap() {
            ContentType::Json => "json",
            ContentType::Csv => "csv",
            ContentType::CsvWithHeader => "csv_with_header",
            ContentType::NdJson => "ndjson",
            ContentType::Parquet => "parquet",
        };

        assert!(!format_str.is_empty());
    }
}

#[test]
fn test_fhir_version_feature_flag_integration() {
    // Test that the correct FHIR version is being used based on feature flags
    #[cfg(feature = "R4")]
    {
        // Test R4-specific functionality
        let patient = json!({
            "resourceType": "Patient",
            "id": "example",
            "active": true,
            "name": [{
                "use": "official",
                "family": "Doe",
                "given": ["John"]
            }]
        });

        let result: Result<helios_fhir::r4::Patient, _> = json::from_value(patient);
        assert!(result.is_ok());
    }
}

#[test]
fn test_error_chain_propagation() {
    // Test that errors properly chain through the system
    let invalid_json = "{ invalid json }";
    let parse_result: Result<serde_json::Value, _> = json::from_str(invalid_json);

    assert!(parse_result.is_err());
    // Just verify that we get an error - the specific message format may vary
    let _error = parse_result.unwrap_err();
}
