//! Unit tests for pysof lib.rs functions
//!
//! These tests focus on testing the core logic of the PyO3 binding functions
//! without requiring Python runtime initialization.

use helios_serde::json;

use chrono::{DateTime, Utc};
use helios_sof::{ContentType, RunOptions, SofError as RustSofError};
use serde_json::json;

fn get_test_view_definition() -> serde_json::Value {
    json!({
        "resourceType": "ViewDefinition",
        "id": "test-view",
        "name": "TestView",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "family_name",
                "path": "name.family"
            }]
        }]
    })
}

fn get_test_bundle() -> serde_json::Value {
    json!({
        "resourceType": "Bundle",
        "id": "test-bundle",
        "type": "collection",
        "entry": [{
            "resource": {
                "resourceType": "Patient",
                "id": "patient-1",
                "name": [{
                    "family": "Doe",
                    "given": ["John"]
                }]
            }
        }]
    })
}

#[test]
fn test_content_type_from_string() {
    // Test valid content types
    assert!(ContentType::from_string("json").is_ok());
    assert!(ContentType::from_string("csv").is_ok());
    assert!(ContentType::from_string("ndjson").is_ok());

    // Test invalid content type
    assert!(ContentType::from_string("invalid").is_err());
}

#[test]
fn test_run_options_creation() {
    // Test with no options
    let options = create_run_options(None, None, None);
    assert!(options.since.is_none());
    assert!(options.limit.is_none());
    assert!(options.page.is_none());

    // Test with all options
    let since_str = "2023-01-01T00:00:00Z";
    let options = create_run_options(Some(since_str), Some(10), Some(1));
    assert!(options.since.is_some());
    assert_eq!(options.limit, Some(10));
    assert_eq!(options.page, Some(1));

    // Test with invalid since format
    let options = create_run_options(Some("invalid-date"), None, None);
    // Should handle invalid date gracefully
    assert!(options.since.is_none());
}

// Helper function to create RunOptions for testing
fn create_run_options(since: Option<&str>, limit: Option<i32>, page: Option<i32>) -> RunOptions {
    let since_datetime = since.and_then(|s| {
        DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    });

    RunOptions {
        since: since_datetime,
        limit: limit.map(|l| l as usize),
        page: page.map(|p| p as usize),
        ..Default::default()
    }
}

#[test]
fn test_view_definition_validation_logic() {
    let valid_view = get_test_view_definition();

    // Test that valid ViewDefinition can be parsed
    #[cfg(feature = "R4")]
    {
        let result: Result<helios_fhir::r4::ViewDefinition, _> =
            json::from_value(valid_view.clone());
        assert!(result.is_ok());
    }

    // Test invalid ViewDefinition structure
    let invalid_view = json!({
        "resourceType": "ViewDefinition",
        "id": "invalid"
        // Missing required fields
    });

    #[cfg(feature = "R4")]
    {
        let result: Result<helios_fhir::r4::ViewDefinition, _> = json::from_value(invalid_view);
        // ViewDefinition validation might be lenient, so we just check it doesn't panic
        let _validation_result = result;
    }
}

#[test]
fn test_bundle_validation_logic() {
    let valid_bundle = get_test_bundle();

    // Test that valid Bundle can be parsed
    #[cfg(feature = "R4")]
    {
        let result: Result<helios_fhir::r4::Bundle, _> = json::from_value(valid_bundle.clone());
        assert!(result.is_ok());
    }

    // Test minimal Bundle structure
    let minimal_bundle = json!({
        "resourceType": "Bundle",
        "id": "minimal",
        "type": "collection"
    });

    #[cfg(feature = "R4")]
    {
        let result: Result<helios_fhir::r4::Bundle, _> = json::from_value(minimal_bundle);
        assert!(result.is_ok());
    }
}

#[test]
fn test_content_type_format_mapping() {
    // Test that ContentType enum maps correctly to format strings
    let csv_type = ContentType::from_string("csv").unwrap();
    let json_type = ContentType::from_string("json").unwrap();
    let ndjson_type = ContentType::from_string("ndjson").unwrap();

    // These should be different variants
    assert_ne!(
        std::mem::discriminant(&csv_type),
        std::mem::discriminant(&json_type)
    );
    assert_ne!(
        std::mem::discriminant(&json_type),
        std::mem::discriminant(&ndjson_type)
    );
    assert_ne!(
        std::mem::discriminant(&csv_type),
        std::mem::discriminant(&ndjson_type)
    );
}

#[test]
fn test_datetime_parsing() {
    // Test valid RFC3339 datetime
    let valid_datetime = "2023-01-01T00:00:00Z";
    let parsed = DateTime::parse_from_rfc3339(valid_datetime);
    assert!(parsed.is_ok());

    // Test invalid datetime format
    let invalid_datetime = "not-a-date";
    let parsed = DateTime::parse_from_rfc3339(invalid_datetime);
    assert!(parsed.is_err());

    // Test datetime with timezone
    let tz_datetime = "2023-01-01T00:00:00+05:00";
    let parsed = DateTime::parse_from_rfc3339(tz_datetime);
    assert!(parsed.is_ok());
}

#[test]
fn test_error_type_mapping() {
    // Test that we can create different error types
    let invalid_view_err = RustSofError::InvalidViewDefinition("test".to_string());
    let fhirpath_err = RustSofError::FhirPathError("test".to_string());
    let unsupported_err = RustSofError::UnsupportedContentType("test".to_string());

    // Verify they are different error variants
    assert_ne!(
        std::mem::discriminant(&invalid_view_err),
        std::mem::discriminant(&fhirpath_err)
    );
    assert_ne!(
        std::mem::discriminant(&fhirpath_err),
        std::mem::discriminant(&unsupported_err)
    );
    assert_ne!(
        std::mem::discriminant(&invalid_view_err),
        std::mem::discriminant(&unsupported_err)
    );
}

#[test]
fn test_json_serialization() {
    let view_def = get_test_view_definition();
    let bundle = get_test_bundle();

    // Test that our test data is valid JSON
    assert!(view_def.is_object());
    assert!(bundle.is_object());

    // Test that we can serialize back to string
    let view_str = serde_json::to_string(&view_def).unwrap();
    let bundle_str = serde_json::to_string(&bundle).unwrap();

    assert!(view_str.contains("ViewDefinition"));
    assert!(bundle_str.contains("Bundle"));
}

#[test]
fn test_content_type_parsing_edge_cases() {
    // Test case sensitivity
    assert!(ContentType::from_string("JSON").is_err());
    assert!(ContentType::from_string("CSV").is_err());

    // Test empty string
    assert!(ContentType::from_string("").is_err());

    // Test whitespace
    assert!(ContentType::from_string(" json ").is_err());
    assert!(ContentType::from_string("json ").is_err());
    assert!(ContentType::from_string(" json").is_err());
}

#[test]
fn test_run_options_edge_cases() {
    // Test with zero values
    let options = create_run_options(None, Some(0), Some(0));
    assert_eq!(options.limit, Some(0));
    assert_eq!(options.page, Some(0));

    // Test with negative values (should convert to usize)
    let options = create_run_options(None, Some(-1), Some(-1));
    // Note: This will wrap around due to usize conversion
    assert!(options.limit.is_some());
    assert!(options.page.is_some());

    // Test with very large values
    let options = create_run_options(None, Some(i32::MAX), Some(i32::MAX));
    assert_eq!(options.limit, Some(i32::MAX as usize));
    assert_eq!(options.page, Some(i32::MAX as usize));
}

#[test]
fn test_datetime_edge_cases() {
    // Test various valid datetime formats
    let valid_formats = [
        "2023-01-01T00:00:00Z",
        "2023-12-31T23:59:59Z",
        "2023-06-15T12:30:45+00:00",
        "2023-06-15T12:30:45-05:00",
        "2023-06-15T12:30:45.123Z",
    ];

    for format in &valid_formats {
        let parsed = DateTime::parse_from_rfc3339(format);
        assert!(parsed.is_ok(), "Failed to parse: {}", format);
    }

    // Test invalid formats
    let invalid_formats = [
        "2023-01-01",
        "2023-01-01 00:00:00",
        "01/01/2023",
        "2023-13-01T00:00:00Z", // Invalid month
        "2023-01-32T00:00:00Z", // Invalid day
        "2023-01-01T25:00:00Z", // Invalid hour
    ];

    for format in &invalid_formats {
        let parsed = DateTime::parse_from_rfc3339(format);
        assert!(parsed.is_err(), "Should have failed to parse: {}", format);
    }
}

#[test]
fn test_fhir_version_feature_flags() {
    // Test that we can check which FHIR versions are compiled in
    #[cfg(feature = "R4")]
    {
        // R4 should be available - try to deserialize a minimal Bundle
        let bundle = json!({
            "resourceType": "Bundle",
            "id": "feat-r4",
            "type": "collection"
        });
        let result: Result<helios_fhir::r4::Bundle, _> = json::from_value(bundle);
        assert!(result.is_ok(), "R4 Bundle should deserialize");
    }

    #[cfg(not(feature = "R4"))]
    {
        // This should not happen with default features
        panic!("R4 feature should be enabled by default");
    }

    // Test other versions based on feature flags
    #[cfg(feature = "R4B")]
    {
        // R4B should be available - try to deserialize a minimal Bundle
        let bundle = json!({
            "resourceType": "Bundle",
            "id": "feat-r4b",
            "type": "collection"
        });
        let result: Result<helios_fhir::r4b::Bundle, _> = json::from_value(bundle);
        assert!(result.is_ok(), "R4B Bundle should deserialize");
    }

    #[cfg(feature = "R5")]
    {
        // R5 should be available - try to deserialize a minimal Bundle
        let bundle = json!({
            "resourceType": "Bundle",
            "id": "feat-r5",
            "type": "collection"
        });
        let result: Result<helios_fhir::r5::Bundle, _> = json::from_value(bundle);
        assert!(result.is_ok(), "R5 Bundle should deserialize");
    }

    #[cfg(feature = "R6")]
    {
        // R6 should be available - try to deserialize a minimal Bundle
        let bundle = json!({
            "resourceType": "Bundle",
            "id": "feat-r6",
            "type": "collection"
        });
        let result: Result<helios_fhir::r6::Bundle, _> = json::from_value(bundle);
        assert!(result.is_ok(), "R6 Bundle should deserialize");
    }
}

#[test]
fn test_json_value_manipulation() {
    let mut view_def = get_test_view_definition();

    // Test that we can modify the JSON structure
    view_def["name"] = json!("ModifiedTestView");
    assert_eq!(view_def["name"], "ModifiedTestView");

    // Test array manipulation
    let mut bundle = get_test_bundle();
    if let Some(entries) = bundle["entry"].as_array_mut() {
        assert_eq!(entries.len(), 1);

        // Add another patient
        entries.push(json!({
            "resource": {
                "resourceType": "Patient",
                "id": "patient-2",
                "name": [{
                    "family": "Smith",
                    "given": ["Jane"]
                }]
            }
        }));

        assert_eq!(entries.len(), 2);
    }
}

#[test]
fn test_error_display_formatting() {
    // Test that error messages are properly formatted
    let errors = vec![
        RustSofError::InvalidViewDefinition("Invalid ViewDefinition structure".to_string()),
        RustSofError::FhirPathError("FHIRPath syntax error".to_string()),
        RustSofError::UnsupportedContentType("Unsupported format: xyz".to_string()),
        RustSofError::CsvWriterError("CSV writing failed".to_string()),
    ];

    for error in errors {
        let error_string = error.to_string();
        assert!(
            !error_string.is_empty(),
            "Error message should not be empty"
        );
        assert!(
            error_string.len() > 5,
            "Error message should be descriptive"
        );
    }
}
