//! Unit tests for parameter validation and filtering functionality
//!
//! This module tests the query parameter validation, parsing, and result filtering
//! logic for the $run operation.

use chrono::{DateTime, Utc};
use helios_sof::ContentType;

/// Mock structures to test parameter validation
/// In a real implementation, these would be imported from the models module

#[derive(Debug)]
struct RunQueryParams {
    format: Option<String>,
    header: Option<String>,
    limit: Option<usize>,
    since: Option<String>,
}

#[derive(Debug, Clone)]
struct ValidatedRunParams {
    format: ContentType,
    limit: Option<usize>,
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

    // Validate limit parameter
    let limit = if let Some(c) = params.limit {
        if c == 0 {
            return Err("_limit parameter must be greater than 0".to_string());
        }
        if c > 10000 {
            return Err("_limit parameter cannot exceed 10000".to_string());
        }
        Some(c)
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
        limit,
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
    let content_type_str = format_param.or(accept_header).unwrap_or("application/json");

    // Handle CSV header parameter
    let content_type_str = if content_type_str == "text/csv" {
        match header_param {
            Some("false") => "text/csv;header=false",
            Some("true") | None => "text/csv;header=true",
            _ => {
                return Err(format!(
                    "Invalid header parameter: {}",
                    header_param.unwrap()
                ));
            }
        }
    } else {
        content_type_str
    };

    ContentType::from_string(content_type_str).map_err(|e| e.to_string())
}

/// Mock function to test result filtering
#[allow(dead_code)]
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
    let output_str =
        String::from_utf8(output_data).map_err(|e| format!("Invalid UTF-8 in output: {}", e))?;

    if params.limit.is_none() {
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
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to serialize filtered NDJSON: {}", e))?
                .join("\n");
            Ok(filtered_ndjson.into_bytes())
        }
        _ => Ok(output_str.into_bytes()),
    }
}

/// Mock function to test CSV filtering
fn apply_csv_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    let output_str = String::from_utf8(output_data)
        .map_err(|e| format!("Invalid UTF-8 in CSV output: {}", e))?;

    if params.limit.is_none() {
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

/// Mock function to test limit limiting logic
fn apply_pagination_to_records(records: &mut Vec<serde_json::Value>, params: &ValidatedRunParams) {
    if let Some(limit) = params.limit {
        records.truncate(limit);
    }
}

/// Mock function to test line limit limiting
fn apply_pagination_to_lines(lines: &mut Vec<&str>, params: &ValidatedRunParams) {
    if let Some(limit) = params.limit {
        lines.truncate(limit);
    }
}

// Unit tests

#[test]
fn test_validate_query_params_valid() {
    let params = RunQueryParams {
        format: Some("application/json".to_string()),
        header: None,
        limit: Some(10),
        since: Some("2023-01-01T00:00:00Z".to_string()),
    };

    let result = validate_query_params(&params, None).unwrap();
    assert_eq!(result.format, ContentType::Json);
    assert_eq!(result.limit, Some(10));
    assert!(result.since.is_some());
}

#[test]
fn test_validate_query_params_invalid_limit() {
    let params = RunQueryParams {
        format: None,
        header: None,
        limit: Some(0),
        since: None,
    };

    let result = validate_query_params(&params, None);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("_limit parameter must be greater than 0")
    );
}

#[test]
fn test_validate_query_params_limit_too_large() {
    let params = RunQueryParams {
        format: None,
        header: None,
        limit: Some(50000),
        since: None,
    };

    let result = validate_query_params(&params, None);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("_limit parameter cannot exceed 10000")
    );
}

#[test]
fn test_validate_query_params_invalid_since() {
    let params = RunQueryParams {
        format: None,
        header: None,
        limit: None,
        since: Some("invalid-date".to_string()),
    };

    let result = validate_query_params(&params, None);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("_since parameter must be a valid RFC3339 timestamp")
    );
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
        parse_content_type(None, Some("text/csv"), Some("false")).unwrap(),
        ContentType::Csv
    );

    assert_eq!(
        parse_content_type(None, Some("text/csv"), Some("true")).unwrap(),
        ContentType::CsvWithHeader
    );
}

#[test]
fn test_apply_csv_filtering() {
    let csv_data = "id,name\n1,John\n2,Jane\n3,Bob\n4,Alice\n"
        .as_bytes()
        .to_vec();
    let params = ValidatedRunParams {
        format: ContentType::CsvWithHeader,
        limit: Some(2),
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
fn test_apply_csv_filtering_with_limit() {
    let csv_data = "id,name\n1,John\n2,Jane\n3,Bob\n4,Alice\n"
        .as_bytes()
        .to_vec();
    let params = ValidatedRunParams {
        format: ContentType::CsvWithHeader,
        limit: Some(2),
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
fn test_apply_json_filtering() {
    let json_data =
        r#"[{"id":"1","name":"John"},{"id":"2","name":"Jane"},{"id":"3","name":"Bob"}]"#
            .as_bytes()
            .to_vec();
    let params = ValidatedRunParams {
        format: ContentType::Json,
        limit: Some(2),
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
{"id":"4","name":"Alice"}"#
        .as_bytes()
        .to_vec();

    let params = ValidatedRunParams {
        format: ContentType::NdJson,
        limit: Some(2),
        since: None,
    };

    let result = apply_ndjson_filtering(ndjson_data, &params).unwrap();
    let result_str = String::from_utf8(result).unwrap();
    let lines: Vec<&str> = result_str.trim().split('\n').collect();

    assert_eq!(lines.len(), 2);
    let first_record: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let second_record: serde_json::Value = serde_json::from_str(lines[1]).unwrap();

    // Should be first 2 records
    assert_eq!(first_record["id"], "1");
    assert_eq!(second_record["id"], "2");
}

// Helper function for NDJSON filtering test
fn apply_ndjson_filtering(
    output_data: Vec<u8>,
    params: &ValidatedRunParams,
) -> Result<Vec<u8>, String> {
    let params_json = ValidatedRunParams {
        format: ContentType::NdJson,
        limit: params.limit,
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
        limit: Some(5),
        since: None,
    };

    apply_pagination_to_records(&mut empty_records, &params);
    assert_eq!(empty_records.len(), 0);

    // Test limit limiting with more records than limit
    let mut records = vec![
        serde_json::json!({"id": "1"}),
        serde_json::json!({"id": "2"}),
        serde_json::json!({"id": "3"}),
        serde_json::json!({"id": "4"}),
    ];

    let params_limit = ValidatedRunParams {
        format: ContentType::Json,
        limit: Some(2),
        since: None,
    };

    apply_pagination_to_records(&mut records, &params_limit);
    assert_eq!(records.len(), 2);
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
            limit: None,
            since: Some(timestamp.to_string()),
        };

        let result = validate_query_params(&params, None);
        assert!(
            result.is_ok(),
            "Failed to parse valid timestamp: {}",
            timestamp
        );
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
            limit: None,
            since: Some(timestamp.to_string()),
        };

        let result = validate_query_params(&params, None);
        assert!(
            result.is_err(),
            "Should have failed to parse invalid timestamp: {}",
            timestamp
        );
    }
}
