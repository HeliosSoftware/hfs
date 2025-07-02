//! Tests for header parameter in the request body

use axum::http::StatusCode;
use serde_json::json;

mod common;

#[tokio::test]
async fn test_header_parameter_boolean_true() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "header",
                "valueBoolean": true
            },
            {
                "name": "_format",
                "valueCode": "text/csv"
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"},
                            {"name": "gender", "path": "gender"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "test-1",
                    "gender": "male"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(
        response.header("content-type").to_str().unwrap(),
        "text/csv"
    );

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.lines().collect();

    // Should have column headers when valueBoolean is true
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("id") && lines[0].contains("gender"));
    assert!(lines[1].contains("test-1") && lines[1].contains("male"));
}

#[tokio::test]
async fn test_header_parameter_boolean_false() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "header",  // Test without underscore
                "valueBoolean": false
            },
            {
                "name": "_format",
                "valueCode": "text/csv"
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"},
                            {"name": "birthDate", "path": "birthDate"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "test-2",
                    "birthDate": "1990-01-01"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(
        response.header("content-type").to_str().unwrap(),
        "text/csv"
    );

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.lines().collect();

    // Should NOT have column headers when valueBoolean is false
    assert_eq!(lines.len(), 1);
    assert!(!lines[0].contains("id,birthDate")); // No column header row
    assert!(lines[0].contains("test-2") && lines[0].contains("1990-01-01"));
}

#[tokio::test]
async fn test_header_parameter_overrides_query() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "header",
                "valueBoolean": false  // Body says no header
            },
            {
                "name": "_format",
                "valueCode": "text/csv"
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "test-3"
                }
            }
        ]
    });

    // Query parameter says true, but body should override
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("header", "true")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.lines().collect();

    // Body parameter (false) should override query parameter (true)
    assert_eq!(lines.len(), 1);
    assert!(!lines[0].contains("id")); // No column headers
    assert!(lines[0].contains("test-3"));
}

#[tokio::test]
async fn test_header_parameter_without_format() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "header",
                "valueBoolean": true
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "test-4"
                }
            }
        ]
    });

    // No format specified, should default to JSON
    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    // header parameter only applies to CSV, should get error with JSON format
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("Header parameter only applies to CSV format")
    );
}

#[tokio::test]
async fn test_header_parameter_with_csv_accept() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "header",
                "valueBoolean": false
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "test-5"
                }
            }
        ]
    });

    // Accept header requests CSV
    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Accept", "text/csv")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(
        response.header("content-type").to_str().unwrap(),
        "text/csv"
    );

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.lines().collect();

    // header parameter false should remove column headers even with CSV accept
    assert_eq!(lines.len(), 1);
    assert!(!lines[0].contains("id"));
    assert!(lines[0].contains("test-5"));
}

#[tokio::test]
async fn test_invalid_header_parameter_type() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "header",
                "valueString": "yes"  // Should be boolean, not string
            },
            {
                "name": "_format",
                "valueCode": "text/csv"
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "test-6"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    // Should get bad request for non-boolean header value
    let status = response.status_code();
    let json: serde_json::Value = response.json();

    if status != StatusCode::BAD_REQUEST {
        // Debug output to understand what happened
        eprintln!("Unexpected response status: {}", status);
        eprintln!(
            "Response body: {}",
            serde_json::to_string_pretty(&json).unwrap()
        );
    }
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("Header parameter must be a boolean value")
    );
}

#[tokio::test]
async fn test_both_format_and_header_in_body() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_format",
                "valueCode": "text/csv"
            },
            {
                "name": "header",
                "valueBoolean": true
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"},
                            {"name": "active", "path": "active"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "test-7",
                    "active": true
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    assert_eq!(
        response.header("content-type").to_str().unwrap(),
        "text/csv"
    );

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.lines().collect();

    // Both format and header specified in body
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("id") && lines[0].contains("active"));
    assert!(lines[1].contains("test-7") && lines[1].contains("true"));
}
