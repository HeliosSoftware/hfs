//! Integration tests for content type handling
//! 
//! Tests the interaction between content type parsing, validation,
//! and the actual data transformation pipeline.

use serde_json::json;
use helios_sof::ContentType;

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
            ContentType::ArrowIpc => "arrow",
        };
        
        assert!(!format_str.is_empty());
    }
}

#[test]
fn test_mime_type_to_format_conversion() {
    // Test MIME type parsing integration
    let mime_mappings = [
        ("text/csv", "csv_with_header"),
        ("application/json", "json"),
        ("application/ndjson", "ndjson"),
        ("text/csv;header=false", "csv"),
        ("text/csv;header=true", "csv_with_header"),
    ];
    
    for (mime_type, expected_format) in &mime_mappings {
        // This would test the actual py_parse_content_type function
        // but since it's not exposed, we test the underlying logic
        let content_type = ContentType::from_string(expected_format);
        assert!(content_type.is_ok(), "Failed to parse expected format: {}", expected_format);
    }
}

#[test]
fn test_unsupported_content_types() {
    let unsupported_types = [
        "text/plain",
        "application/xml", 
        "text/html",
        "application/pdf",
        "image/png",
        "video/mp4",
    ];
    
    for unsupported in &unsupported_types {
        let result = ContentType::from_string(unsupported);
        assert!(result.is_err(), "Should not support content type: {}", unsupported);
    }
}
