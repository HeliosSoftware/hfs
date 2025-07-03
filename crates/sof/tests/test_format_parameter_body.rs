//! Tests for _format parameter in the request body

use axum::http::StatusCode;
use serde_json::json;

mod common;

#[tokio::test]
async fn test_format_parameter_in_body_csv() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "id": "example",
        "parameter": [
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
                            {"name": "id", "path": "getResourceKey()"},
                            {"name": "birthDate", "path": "birthDate"},
                            {"name": "family", "path": "name.family"},
                            {"name": "given", "path": "name.given"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-1",
                    "name": [{
                        "use": "official",
                        "family": "Cole",
                        "given": ["Joanie"]
                    }],
                    "birthDate": "2012-03-30"
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-2",
                    "name": [{
                        "use": "official",
                        "family": "Doe",
                        "given": ["John"]
                    }],
                    "birthDate": "2012-03-30"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "text/csv");

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.lines().collect();

    assert!(lines.len() >= 3); // Header + 2 data rows
    assert!(
        lines[0].contains("id")
            && lines[0].contains("birthDate")
            && lines[0].contains("family")
            && lines[0].contains("given")
    );
    assert!(
        lines[1].contains("pt-1")
            && lines[1].contains("2012-03-30")
            && lines[1].contains("Cole")
            && lines[1].contains("Joanie")
    );
    assert!(
        lines[2].contains("pt-2")
            && lines[2].contains("2012-03-30")
            && lines[2].contains("Doe")
            && lines[2].contains("John")
    );
}

#[tokio::test]
async fn test_format_parameter_in_body_overrides_accept_header() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
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
                    "id": "example",
                    "gender": "male"
                }
            }
        ]
    });

    // Send request with Accept header for JSON, but _format in body should override
    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Accept", "application/json")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(
        content_type.to_str().unwrap(),
        "text/csv",
        "_format parameter should override Accept header"
    );

    let csv_text = response.text();
    assert!(csv_text.contains("id,gender") || csv_text.contains("id\tgender"));
}

#[tokio::test]
async fn test_format_parameter_in_body_overrides_query_parameter() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
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
                    "id": "test"
                }
            }
        ]
    });

    // Send request with query parameter for JSON, but _format in body should override
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_format", "application/json")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(
        content_type.to_str().unwrap(),
        "text/csv",
        "_format parameter in body should override query parameter"
    );
}

#[tokio::test]
async fn test_format_parameter_valuestring_variant() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_format",
                "valueString": "application/ndjson"  // Using valueString instead of valueCode
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
                    "id": "1",
                    "active": true
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "2",
                    "active": false
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "application/ndjson");

    let ndjson_text = response.text();
    let lines: Vec<&str> = ndjson_text.trim().lines().collect();

    assert_eq!(lines.len(), 2);
    let line1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let line2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();

    assert_eq!(line1["id"], "1");
    assert_eq!(line1["active"], true);
    assert_eq!(line2["id"], "2");
    assert_eq!(line2["active"], false);
}

#[tokio::test]
async fn test_format_parameter_with_csv_header_control() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
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
                    "id": "test"
                }
            }
        ]
    });

    // Test with header=false query parameter
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("header", "false")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "text/csv");

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.lines().collect();

    // Should have only 1 line (no header)
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("test"));
    assert!(
        !lines[0].contains("id"),
        "Header should not be present when header=false"
    );
}

#[tokio::test]
async fn test_invalid_format_parameter_in_body() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_format",
                "valueCode": "text/plain"  // Invalid format
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [{"name": "id", "path": "id"}]
                    }]
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("Unsupported content type")
    );
}

#[tokio::test]
async fn test_precedence_order_body_query_accept() {
    let server = common::test_server().await;

    // Test that precedence is: body _format > query _format > Accept header
    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_format",
                "valueCode": "application/ndjson"  // Body parameter (highest priority)
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [{"name": "id", "path": "id"}]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "test"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_format", "text/csv") // Query parameter (medium priority)
        .add_header("Accept", "application/json") // Accept header (lowest priority)
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(
        content_type.to_str().unwrap(),
        "application/ndjson",
        "Body _format parameter should have highest precedence"
    );
}
