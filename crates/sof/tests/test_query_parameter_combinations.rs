//! Tests for query parameter combinations and edge cases

use axum::http::StatusCode;
use serde_json::json;

mod common;

#[tokio::test]
async fn test_pagination_parameters_combined() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{
                        "column": [{
                            "name": "id",
                            "path": "id"
                        }]
                    }]
                }
            },
            // Add 5 resources to test pagination
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "1"
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "2"
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "3"
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "4"
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "5"
                }
            }
        ]
    });

    // Test count 2 - should only return first 2 records
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_limit", "2")
        .add_query_param("_format", "application/json")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    let rows = json.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "1");
    assert_eq!(rows[1]["id"], "2");

    // Test count 3 - should return first 3 records
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_limit", "3")
        .add_query_param("_format", "application/json")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    let rows = json.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["id"], "1");
    assert_eq!(rows[1]["id"], "2");
    assert_eq!(rows[2]["id"], "3");

    // Test count 5 - should return all 5 records
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_limit", "5")
        .add_query_param("_format", "application/json")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    let rows = json.as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0]["id"], "1");
    assert_eq!(rows[4]["id"], "5");
}

#[tokio::test]
async fn test_limit_parameter_boundaries() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "viewResource",
            "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{"column": [{"name": "id", "path": "id"}]}]
            }
        }]
    });

    // Test _limit = 1 (minimum valid)
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_limit", "1")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    // Test _limit = 10000 (maximum valid)
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_limit", "10000")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);

    // Test _limit = 0 (invalid)
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_limit", "0")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("_limit parameter must be greater than 0")
    );

    // Test _limit = 10001 (too large)
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_limit", "10001")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = response.json();
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("_limit parameter cannot exceed 10000")
    );
}

#[tokio::test]
async fn test_format_and_accept_header_precedence() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "viewResource",
            "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{"column": [{"name": "id", "path": "id"}]}]
            }
        }, {
            "name": "resource",
            "resource": {
                "resourceType": "Patient",
                "id": "test"
            }
        }]
    });

    // Test that _format parameter takes precedence over Accept header
    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Accept", "application/json")
        .add_query_param("_format", "text/csv")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "text/csv");
}

#[tokio::test]
async fn test_csv_header_parameter_with_non_csv_format() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "viewResource",
            "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{"column": [{"name": "id", "path": "id"}]}]
            }
        }]
    });

    // header parameter should be ignored for non-CSV formats
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_format", "application/json")
        .add_query_param("header", "absent")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "application/json");
}

#[tokio::test]
async fn test_invalid_since_parameter() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "viewResource",
            "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{"column": [{"name": "id", "path": "id"}]}]
            }
        }]
    });

    // Test with invalid RFC3339 timestamp
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_since", "not-a-date")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = response.json();
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("_since parameter must be a valid RFC3339 timestamp")
    );
}

#[tokio::test]
async fn test_since_parameter_query_filtering() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "viewResource",
            "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{
                    "column": [
                        {"name": "id", "path": "id"},
                        {"name": "lastUpdated", "path": "meta.lastUpdated"}
                    ]
                }]
            }
        }, {
            "name": "resource",
            "resource": {
                "resourceType": "Patient",
                "id": "old-patient",
                "meta": {
                    "lastUpdated": "2023-01-01T00:00:00Z"
                }
            }
        }, {
            "name": "resource",
            "resource": {
                "resourceType": "Patient",
                "id": "new-patient",
                "meta": {
                    "lastUpdated": "2023-12-01T00:00:00Z"
                }
            }
        }]
    });

    // Test with _since as query parameter
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_since", "2023-06-01T00:00:00Z")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    let results = json.as_array().unwrap();

    // Should only return the patient updated after 2023-06-01
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["id"], "new-patient");
}

#[tokio::test]
async fn test_valid_since_parameter_formats() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "viewResource",
            "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{"column": [{"name": "id", "path": "id"}]}]
            }
        }]
    });

    // Test with various valid RFC3339 formats
    let valid_formats = vec![
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:00+00:00",
        "2024-01-01T00:00:00-05:00",
        "2024-01-01T12:30:45.123Z",
    ];

    for timestamp in valid_formats {
        let response = server
            .post("/ViewDefinition/$run")
            .add_query_param("_since", timestamp)
            .json(&request_body)
            .await;

        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "Failed for timestamp: {}",
            timestamp
        );
    }
}

#[tokio::test]
async fn test_combined_filtering_parameters() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "viewResource",
            "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{"column": [{"name": "id", "path": "id"}]}]
            }
        }]
    });

    // Test all filtering parameters together
    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_format", "application/json")
        .add_query_param("_limit", "50")
        .add_query_param("_since", "2024-01-01T00:00:00Z")
        .add_query_param("patient", "Patient/123")
        .add_query_param("group", "Group/456")
        .add_query_param("source", "primary-database")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
}
