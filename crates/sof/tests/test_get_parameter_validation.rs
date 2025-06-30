//! Tests for GET parameter validation per FHIR specification constraints

use axum::http::StatusCode;
use serde_json::json;

mod common;

#[tokio::test]
async fn test_get_with_view_reference_fails() {
    let server = common::test_server().await;
    
    let response = server
        .get("/ViewDefinition/$run")
        .add_query_param("viewReference", "ViewDefinition/123")
        .add_query_param("_format", "json")
        .await;
    
    let status = response.status_code();
    let json: serde_json::Value = response.json();
    
    if status != StatusCode::BAD_REQUEST {
        eprintln!("Unexpected status: {}", status);
        eprintln!("Response: {}", serde_json::to_string_pretty(&json).unwrap());
    }
    
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(json["issue"][0]["details"]["text"]
        .as_str()
        .unwrap()
        .contains("GET operations cannot use complex parameters like viewReference"));
}

#[tokio::test]
async fn test_get_with_patient_parameter_fails() {
    let server = common::test_server().await;
    
    let response = server
        .get("/ViewDefinition/$run")
        .add_query_param("patient", "Patient/123")
        .add_query_param("_format", "json")
        .await;
    
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(json["issue"][0]["details"]["text"]
        .as_str()
        .unwrap()
        .contains("GET operations cannot use complex parameters like patient"));
}

#[tokio::test]
async fn test_get_with_group_parameter_fails() {
    let server = common::test_server().await;
    
    let response = server
        .get("/ViewDefinition/$run")
        .add_query_param("group", "Group/456")
        .add_query_param("_format", "json")
        .await;
    
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(json["issue"][0]["details"]["text"]
        .as_str()
        .unwrap()
        .contains("GET operations cannot use complex parameters like group"));
}

#[tokio::test]
async fn test_get_with_simple_parameters_only() {
    let server = common::test_server().await;
    
    // Even with only simple parameters, GET still fails because we need a ViewDefinition
    let response = server
        .get("/ViewDefinition/$run")
        .add_query_param("_format", "json")
        .add_query_param("_count", "100")
        .add_query_param("_page", "1")
        .add_query_param("header", "true")
        .await;
    
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    // Should get error about needing ViewDefinition, not about parameters
    assert!(json["issue"][0]["details"]["text"]
        .as_str()
        .unwrap()
        .contains("GET /ViewDefinition/$run requires a ViewDefinition"));
}

#[tokio::test]
async fn test_post_with_complex_parameters_succeeds() {
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
                        "column": [
                            {"name": "id", "path": "id"}
                        ]
                    }]
                }
            },
            {
                "name": "patient",
                "valueReference": {
                    "reference": "Patient/123"
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "123"
                }
            }
        ]
    });
    
    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;
    
    // POST should succeed with complex parameters
    assert_eq!(response.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_explains_to_use_post() {
    let server = common::test_server().await;
    
    let response = server
        .get("/ViewDefinition/$run")
        .add_query_param("viewReference", "ViewDefinition/test")
        .await;
    
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    
    let json: serde_json::Value = response.json();
    assert!(json["issue"][0]["details"]["text"]
        .as_str()
        .unwrap()
        .contains("Use POST instead"));
}

#[tokio::test]
async fn test_get_with_source_parameter_fails() {
    let server = common::test_server().await;
    
    let response = server
        .get("/ViewDefinition/$run")
        .add_query_param("source", "my-data-source")
        .add_query_param("_format", "json")
        .await;
    
    let status = response.status_code();
    let json: serde_json::Value = response.json();
    
    if status != StatusCode::BAD_REQUEST {
        eprintln!("Unexpected status: {}", status);
        eprintln!("Response: {}", serde_json::to_string_pretty(&json).unwrap());
    }
    
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(json["issue"][0]["details"]["text"]
        .as_str()
        .unwrap()
        .contains("GET operations cannot use the source parameter"));
}