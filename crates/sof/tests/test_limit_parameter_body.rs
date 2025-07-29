//! Tests for _limit parameter in the request body
//!
//! This module tests that the _limit parameter works when provided
//! in the Parameters resource body, not just as a query parameter.

use axum::http::StatusCode;
use serde_json::json;

mod common;

/// Helper to create a ViewDefinition for patient data
fn patient_view_definition() -> serde_json::Value {
    json!({
        "resourceType": "ViewDefinition",
        "id": "patient-demographics",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "family",
                "path": "name.family"
            }]
        }]
    })
}

/// Helper to create test patients bundle
fn test_patients_bundle() -> serde_json::Value {
    json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-1",
                    "name": [{
                        "family": "Smith",
                        "given": ["John"]
                    }]
                }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-2",
                    "name": [{
                        "family": "Jones",
                        "given": ["Jane"]
                    }]
                }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-3",
                    "name": [{
                        "family": "Brown",
                        "given": ["Bob"]
                    }]
                }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-4",
                    "name": [{
                        "family": "Davis",
                        "given": ["David"]
                    }]
                }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-5",
                    "name": [{
                        "family": "Wilson",
                        "given": ["Will"]
                    }]
                }
            }
        ]
    })
}

/// Test _limit parameter with valueInteger in request body
#[tokio::test]
async fn test_limit_parameter_in_body_value_integer() {
    let server = common::test_server().await;

    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": patient_view_definition()
            },
            {
                "name": "resource",
                "resource": test_patients_bundle()
            },
            {
                "name": "_limit",
                "valueInteger": 2
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .add_header("Accept", "application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::OK);

    let result = response.json::<serde_json::Value>();

    // Should return only 2 patients due to _limit limit
    assert!(result.is_array());
    let patients = result.as_array().unwrap();
    assert_eq!(
        patients.len(),
        2,
        "Expected 2 patients due to _limit=2, got {}",
        patients.len()
    );
    assert_eq!(patients[0]["id"], "pt-1");
    assert_eq!(patients[1]["id"], "pt-2");
}

/// Test _limit parameter with valuePositiveInt in request body
#[tokio::test]
async fn test_limit_parameter_in_body_value_positive_int() {
    let server = common::test_server().await;

    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": patient_view_definition()
            },
            {
                "name": "resource",
                "resource": test_patients_bundle()
            },
            {
                "name": "_limit",
                "valuePositiveInt": 3
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .add_header("Accept", "application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::OK);

    let result = response.json::<serde_json::Value>();

    // Should return only 3 patients due to _limit limit
    assert!(result.is_array());
    let patients = result.as_array().unwrap();
    assert_eq!(
        patients.len(),
        3,
        "Expected 3 patients due to _limit=3, got {}",
        patients.len()
    );
}

/// Test _limit parameter validation - negative value
#[tokio::test]
async fn test_limit_parameter_negative_value() {
    let server = common::test_server().await;

    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": patient_view_definition()
            },
            {
                "name": "resource",
                "resource": test_patients_bundle()
            },
            {
                "name": "_limit",
                "valueInteger": -1
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}

/// Test _limit parameter validation - exceeds maximum
#[tokio::test]
async fn test_limit_parameter_exceeds_maximum() {
    let server = common::test_server().await;

    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": patient_view_definition()
            },
            {
                "name": "resource",
                "resource": test_patients_bundle()
            },
            {
                "name": "_limit",
                "valueInteger": 10001
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::BAD_REQUEST);
}

/// Test _limit parameter with CSV output
#[tokio::test]
async fn test_limit_parameter_with_csv_format() {
    let server = common::test_server().await;

    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": patient_view_definition()
            },
            {
                "name": "resource",
                "resource": test_patients_bundle()
            },
            {
                "name": "_limit",
                "valueInteger": 2
            },
            {
                "name": "_format",
                "valueCode": "text/csv"
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::OK);

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.trim().split('\n').collect();

    // Should have header + 2 data rows
    assert_eq!(
        lines.len(),
        3,
        "Expected 3 lines (header + 2 data rows), got {}",
        lines.len()
    );
    assert!(lines[0].contains("id"));
    assert!(lines[1].contains("pt-1"));
    assert!(lines[2].contains("pt-2"));
}

/// Test that _count parameter in body takes precedence over query parameter
#[tokio::test]
async fn test_limit_parameter_body_overrides_query() {
    let server = common::test_server().await;

    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": patient_view_definition()
            },
            {
                "name": "resource",
                "resource": test_patients_bundle()
            },
            {
                "name": "_limit",
                "valueInteger": 2
            }
        ]
    });

    // Query parameter says 5, but body says 2
    let response = server
        .post("/ViewDefinition/$run?_count=5")
        .content_type("application/json")
        .add_header("Accept", "application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::OK);

    let result = response.json::<serde_json::Value>();

    // Should return only 2 patients (body parameter takes precedence)
    assert!(result.is_array());
    let patients = result.as_array().unwrap();
    assert_eq!(
        patients.len(),
        2,
        "Body parameter should override query parameter"
    );
}
