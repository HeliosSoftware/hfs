//! Unit tests for parameter validation and filtering functionality
//!
//! This module tests the query parameter validation, parsing, and result filtering
//! logic for the $run operation.

use chrono::{DateTime, Utc};
use sof::ContentType;

/// Mock structures to test parameter validation
/// In a real implementation, these would be imported from the models module

#[derive(Debug)]
struct RunQueryParams {
    format: Option<String>,
    header: Option<String>,
    count: Option<usize>,
    page: Option<usize>,
    since: Option<String>,
}

#[derive(Debug, Clone)]
struct ValidatedRunParams {
    format: ContentType,
    count: Option<usize>,
    page: Option<usize>,
    since: Option<DateTime<Utc>>,
}

/// Mock function to test parameter validation logic
fn validate_query_params(
    params: &RunQueryParams,
    accept_header: Option<&str>,
) -> Result<ValidatedRunParams, String> {
    // Parse content type
    let format = parse_content_type(
        accept_header,
        params.format.as_deref(),
        params.header.as_deref(),
    )?;
    
    // Validate count parameter
    let count = if let Some(c) = params.count {
        if c == 0 {
            return Err("_count parameter must be greater than 0".to_string());
        }
        if c > 10000 {
            return Err("_count parameter cannot exceed 10000".to_string());
        }
        Some(c)
    } else {
        None
    };
    
    // Validate page parameter
    let page = if let Some(p) = params.page {
        if p == 0 {
            return Err("_page parameter must be greater than 0 (1-based)".to_string());
        }
        Some(p)
    } else {
        None
    };
    
    // Validate since parameter
    let since = if let Some(since_str) = &params.since {
        match DateTime::parse_from_rfc3339(since_str) {
            Ok(dt) => Some(dt.with_timezone(&Utc)),
            Err(_) => {
                return Err(format!(
                    "_since parameter must be a valid RFC3339 timestamp: {}",
                    since_str
                ));
            }
        }
    } else {
        None
    };
    
    Ok(ValidatedRunParams {
        format,
        count,
        page,
        since,
    })
}

/// Mock function to test content type parsing
fn parse_content_type(
    accept_header: Option<&str>,
    format_param: Option<&str>,
    header_param: Option<&str>,
) -> Result<ContentType, String> {
    // Query parameter takes precedence over Accept header
    let content_type_str = format_param
        .or(accept_header)
        .unwrap_or("application/json");
    
    // Handle CSV header parameter
    let content_type_str = if content_type_str == "text/csv" {
        match header_param {
            Some("absent") => "text/csv",
            Some("present") | None => "text/csv;header=present",
            _ => return Err(format!("Invalid _header parameter: {}", header_param.unwrap())),
        }
    } else {
        content_type_str
    };
    
    ContentType::from_string(content_type_str)
        .map_err(|e| e.to_string())
}

/// Mock function to test result filtering
fn apply_result_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    match params.format {
        ContentType::Json | ContentType::NdJson => apply_json_filtering(output_data, params),
        ContentType::Csv | ContentType::CsvWithHeader => apply_csv_filtering(output_data, params),
        ContentType::Parquet => {
            // Parquet filtering is not implemented in this scope
            Ok(output_data)
        }
    }
}

/// Mock function to test JSON filtering
fn apply_json_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    let output_str = String::from_utf8(output_data)
        .map_err(|e| format!("Invalid UTF-8 in output: {}", e))?;
    
    if params.count.is_none() && params.page.is_none() {
        return Ok(output_str.into_bytes());
    }
    
    match params.format {
        ContentType::Json => {
            // Parse as JSON array and apply pagination
            let mut records: Vec<serde_json::Value> = serde_json::from_str(&output_str)
                .map_err(|e| format!("Invalid JSON output: {}", e))?;
            
            apply_pagination_to_records(&mut records, params);
            
            let filtered_json = serde_json::to_string(&records)
                .map_err(|e| format!("Failed to serialize filtered JSON: {}", e))?;
            Ok(filtered_json.into_bytes())
        }
        ContentType::NdJson => {
            // Parse as NDJSON and apply pagination
            let mut records = Vec::new();
            for line in output_str.lines() {
                if !line.trim().is_empty() {
                    let record: serde_json::Value = serde_json::from_str(line)
                        .map_err(|e| format!("Invalid NDJSON line: {}", e))?;
                    records.push(record);
                }
            }
            
            apply_pagination_to_records(&mut records, params);
            
            let filtered_ndjson = records
                .iter()
                .map(|r| serde_json::to_string(r))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to serialize filtered NDJSON: {}", e))?
                .join("\n");
            Ok(filtered_ndjson.into_bytes())
        }
        _ => Ok(output_str.into_bytes())
    }
}

/// Mock function to test CSV filtering
fn apply_csv_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    let output_str = String::from_utf8(output_data)
        .map_err(|e| format!("Invalid UTF-8 in CSV output: {}", e))?;
    
    if params.count.is_none() && params.page.is_none() {
        return Ok(output_str.into_bytes());
    }
    
    let lines: Vec<&str> = output_str.lines().collect();
    if lines.is_empty() {
        return Ok(output_str.into_bytes());
    }
    
    // Check if we have headers based on the format
    let has_header = matches!(params.format, ContentType::CsvWithHeader);
    let header_offset = if has_header { 1 } else { 0 };
    
    if lines.len() <= header_offset {
        return Ok(output_str.into_bytes());
    }
    
    // Split into header and data lines
    let (header_lines, data_lines) = if has_header {
        (lines[0..1].to_vec(), lines[1..].to_vec())
    } else {
        (Vec::new(), lines)
    };
    
    // Apply pagination to data lines
    let mut data_lines = data_lines;
    apply_pagination_to_lines(&mut data_lines, params);
    
    // Reconstruct CSV
    let mut result_lines = header_lines;
    result_lines.extend(data_lines);
    let result = result_lines.join("\n");
    
    // Add final newline if original had one
    if output_str.ends_with('\n') && !result.ends_with('\n') {
        Ok(format!("{}\n", result).into_bytes())
    } else {
        Ok(result.into_bytes())
    }
}

/// Mock function to test pagination logic
fn apply_pagination_to_records(records: &mut Vec<serde_json::Value>, params: &ValidatedRunParams) {
    let count = params.count.unwrap_or(records.len());
    let page = params.page.unwrap_or(1);
    
    // Calculate offset based on page (1-based)
    let offset = (page - 1) * count;
    
    if offset >= records.len() {
        records.clear();
        return;
    }
    
    let end = std::cmp::min(offset + count, records.len());
    *records = records[offset..end].to_vec();
}

/// Mock function to test line pagination
fn apply_pagination_to_lines(lines: &mut Vec<&str>, params: &ValidatedRunParams) {
    let count = params.count.unwrap_or(lines.len());
    let page = params.page.unwrap_or(1);
    
    // Calculate offset based on page (1-based)
    let offset = (page - 1) * count;
    
    if offset >= lines.len() {
        lines.clear();
        return;
    }
    
    let end = std::cmp::min(offset + count, lines.len());
    *lines = lines[offset..end].to_vec();
}

// Unit tests

#[test]
fn test_validate_query_params_valid() {
    let params = RunQueryParams {
        format: Some("application/json".to_string()),
        header: None,
        count: Some(10),
        page: Some(2),
        since: Some("2023-01-01T00:00:00Z".to_string()),
    };
    
    let result = validate_query_params(&params, None).unwrap();
    assert_eq!(result.format, ContentType::Json);
    assert_eq!(result.count, Some(10));
    assert_eq!(result.page, Some(2));
    assert!(result.since.is_some());
}

#[test]
fn test_validate_query_params_invalid_count() {
    let params = RunQueryParams {
        format: None,
        header: None,
        count: Some(0),
        page: None,
        since: None,
    };
    
    let result = validate_query_params(&params, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("_count parameter must be greater than 0"));
}

#[test]
fn test_validate_query_params_count_too_large() {
    let params = RunQueryParams {
        format: None,
        header: None,
        count: Some(50000),
        page: None,
        since: None,
    };
    
    let result = validate_query_params(&params, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("_count parameter cannot exceed 10000"));
}

#[test]
fn test_validate_query_params_invalid_page() {
    let params = RunQueryParams {
        format: None,
        header: None,
        count: None,
        page: Some(0),
        since: None,
    };
    
    let result = validate_query_params(&params, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("_page parameter must be greater than 0"));
}

#[test]
fn test_validate_query_params_invalid_since() {
    let params = RunQueryParams {
        format: None,
        header: None,
        count: None,
        page: None,
        since: Some("invalid-date".to_string()),
    };
    
    let result = validate_query_params(&params, None);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("_since parameter must be a valid RFC3339 timestamp"));
}

#[test]
fn test_parse_content_type_accept_header() {
    assert_eq!(
        parse_content_type(Some("text/csv"), None, None).unwrap(),
        ContentType::CsvWithHeader
    );
}

#[test]
fn test_parse_content_type_format_override() {
    assert_eq!(
        parse_content_type(Some("text/csv"), Some("application/json"), None).unwrap(),
        ContentType::Json
    );
}

#[test]
fn test_parse_content_type_csv_header_control() {
    assert_eq!(
        parse_content_type(None, Some("text/csv"), Some("absent")).unwrap(),
        ContentType::Csv
    );
    
    assert_eq!(
        parse_content_type(None, Some("text/csv"), Some("present")).unwrap(),
        ContentType::CsvWithHeader
    );
}

#[test]
fn test_apply_csv_filtering() {
    let csv_data = "id,name\n1,John\n2,Jane\n3,Bob\n4,Alice\n".as_bytes().to_vec();
    let params = ValidatedRunParams {
        format: ContentType::CsvWithHeader,
        count: Some(2),
        page: Some(1),
        since: None,
    };
    
    let result = apply_csv_filtering(csv_data, &params).unwrap();
    let result_str = String::from_utf8(result).unwrap();
    
    assert!(result_str.contains("id,name"));
    assert!(result_str.contains("1,John"));
    assert!(result_str.contains("2,Jane"));
    assert!(!result_str.contains("3,Bob"));
    assert!(!result_str.contains("4,Alice"));
}

#[test]
fn test_apply_csv_filtering_page_2() {
    let csv_data = "id,name\n1,John\n2,Jane\n3,Bob\n4,Alice\n".as_bytes().to_vec();
    let params = ValidatedRunParams {
        format: ContentType::CsvWithHeader,
        count: Some(2),
        page: Some(2),
        since: None,
    };
    
    let result = apply_csv_filtering(csv_data, &params).unwrap();
    let result_str = String::from_utf8(result).unwrap();
    
    assert!(result_str.contains("id,name"));
    assert!(!result_str.contains("1,John"));
    assert!(!result_str.contains("2,Jane"));
    assert!(result_str.contains("3,Bob"));
    assert!(result_str.contains("4,Alice"));
}

#[test]
fn test_apply_json_filtering() {
    let json_data = r#"[{"id":"1","name":"John"},{"id":"2","name":"Jane"},{"id":"3","name":"Bob"}]"#.as_bytes().to_vec();
    let params = ValidatedRunParams {
        format: ContentType::Json,
        count: Some(2),
        page: Some(1),
        since: None,
    };
    
    let result = apply_json_filtering(json_data, &params).unwrap();
    let result_str = String::from_utf8(result).unwrap();
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&result_str).unwrap();
    
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0]["id"], "1");
    assert_eq!(parsed[1]["id"], "2");
}

#[test]
fn test_apply_ndjson_filtering() {
    let ndjson_data = r#"{"id":"1","name":"John"}
{"id":"2","name":"Jane"}
{"id":"3","name":"Bob"}
{"id":"4","name":"Alice"}"#.as_bytes().to_vec();
    
    let params = ValidatedRunParams {
        format: ContentType::NdJson,
        count: Some(2),
        page: Some(2),
        since: None,
    };
    
    let result = apply_ndjson_filtering(ndjson_data, &params).unwrap();
    let result_str = String::from_utf8(result).unwrap();
    let lines: Vec<&str> = result_str.trim().split('\n').collect();
    
    assert_eq!(lines.len(), 2);
    let first_record: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let second_record: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    
    // Should be page 2, so records 3 and 4
    assert_eq!(first_record["id"], "3");
    assert_eq!(second_record["id"], "4");
}

// Helper function for NDJSON filtering test
fn apply_ndjson_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    let params_json = ValidatedRunParams {
        format: ContentType::NdJson,
        count: params.count,
        page: params.page,
        since: params.since,
    };
    apply_json_filtering(output_data, &params_json)
}

#[test]
fn test_pagination_edge_cases() {
    // Test pagination with empty data
    let mut empty_records: Vec<serde_json::Value> = Vec::new();
    let params = ValidatedRunParams {
        format: ContentType::Json,
        count: Some(5),
        page: Some(1),
        since: None,
    };
    
    apply_pagination_to_records(&mut empty_records, &params);
    assert_eq!(empty_records.len(), 0);
    
    // Test pagination beyond available data
    let mut records = vec![
        serde_json::json!({"id": "1"}),
        serde_json::json!({"id": "2"}),
    ];
    
    let params_beyond = ValidatedRunParams {
        format: ContentType::Json,
        count: Some(10),
        page: Some(2),
        since: None,
    };
    
    apply_pagination_to_records(&mut records, &params_beyond);
    assert_eq!(records.len(), 0);
}

#[test]
fn test_since_parameter_parsing() {
    // Valid RFC3339 timestamps
    let valid_timestamps = vec![
        "2023-01-01T00:00:00Z",
        "2023-01-01T12:30:45.123Z",
        "2023-01-01T00:00:00+00:00",
        "2023-01-01T00:00:00-05:00",
    ];
    
    for timestamp in valid_timestamps {
        let params = RunQueryParams {
            format: None,
            header: None,
            count: None,
            page: None,
            since: Some(timestamp.to_string()),
        };
        
        let result = validate_query_params(&params, None);
        assert!(result.is_ok(), "Failed to parse valid timestamp: {}", timestamp);
        assert!(result.unwrap().since.is_some());
    }
    
    // Invalid timestamps
    let invalid_timestamps = vec![
        "2023-01-01",
        "2023-01-01 12:00:00",
        "invalid",
        "2023-13-01T00:00:00Z", // Invalid month
    ];
    
    for timestamp in invalid_timestamps {
        let params = RunQueryParams {
            format: None,
            header: None,
            count: None,
            page: None,
            since: Some(timestamp.to_string()),
        };
        
        let result = validate_query_params(&params, None);
        assert!(result.is_err(), "Should have failed to parse invalid timestamp: {}", timestamp);
    }
}