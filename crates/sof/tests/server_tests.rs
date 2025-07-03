//! Integration tests for the SQL-on-FHIR server

use axum::http::StatusCode;
use serde_json::json;

mod common;

#[tokio::test]
async fn test_health_endpoint() {
    let server = common::test_server().await;

    let response = server.get("/health").await;

    assert_eq!(response.status_code(), StatusCode::OK);

    let json: serde_json::Value = response.json();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "sof-server");
}

#[tokio::test]
async fn test_capability_statement() {
    let server = common::test_server().await;

    let response = server.get("/metadata").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "application/fhir+json");

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "CapabilityStatement");
    assert_eq!(json["kind"], "instance");
    assert_eq!(json["fhirVersion"], "4.0.1");

    // Verify ViewDefinition resource is supported
    let resources = json["rest"][0]["resource"].as_array().unwrap();
    let view_def_resource = resources
        .iter()
        .find(|r| r["type"] == "ViewDefinition")
        .expect("ViewDefinition resource should be listed");

    // Verify $run operation is supported
    let operations = view_def_resource["operation"].as_array().unwrap();
    assert!(operations.iter().any(|op| op["name"] == "run"));
}

#[tokio::test]
async fn test_run_view_definition_basic() {
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
                        }, {
                            "name": "gender",
                            "path": "gender"
                        }]
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

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Accept", "application/json")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "application/json");

    let json: serde_json::Value = response.json();
    assert!(json.is_array());

    let rows = json.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "example");
    assert_eq!(rows[0]["gender"], "male");
}

#[tokio::test]
async fn test_run_view_definition_csv_output() {
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
                        }, {
                            "name": "name",
                            "path": "name.family"
                        }]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "123",
                    "name": [{
                        "family": "Doe",
                        "given": ["John"]
                    }]
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_format", "text/csv")
        .add_query_param("header", "present")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "text/csv");

    let csv_text = response.text();
    let lines: Vec<&str> = csv_text.lines().collect();

    assert_eq!(lines.len(), 2); // Column headers + 1 data row
    assert_eq!(lines[0], "id,name");
    assert!(lines[1].contains("123"));
    assert!(lines[1].contains("Doe"));
}

#[tokio::test]
async fn test_run_view_definition_ndjson_output() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Observation",
                    "select": [{
                        "column": [{
                            "name": "id",
                            "path": "id"
                        }, {
                            "name": "status",
                            "path": "status"
                        }]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Observation",
                    "id": "obs1",
                    "status": "final"
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Observation",
                    "id": "obs2",
                    "status": "preliminary"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Accept", "application/ndjson")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "application/ndjson");

    let ndjson_text = response.text();
    let lines: Vec<&str> = ndjson_text.trim().lines().collect();

    assert_eq!(lines.len(), 2);

    let row1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let row2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();

    assert_eq!(row1["id"], "obs1");
    assert_eq!(row1["status"], "final");
    assert_eq!(row2["id"], "obs2");
    assert_eq!(row2["status"], "preliminary");
}

#[tokio::test]
async fn test_run_view_definition_error_invalid_parameters() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Bundle",  // Wrong resource type
        "type": "collection"
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert_eq!(json["issue"][0]["severity"], "error");
}

#[tokio::test]
async fn test_run_view_definition_error_no_view() {
    let server = common::test_server().await;

    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": []  // No ViewDefinition provided
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
}

#[tokio::test]
async fn test_run_view_definition_unsupported_format() {
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

    let response = server
        .post("/ViewDefinition/$run")
        .add_query_param("_format", "text/plain") // Unsupported format
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
}

#[tokio::test]
async fn test_run_view_definition_get_without_reference() {
    let server = common::test_server().await;

    // GET request without viewReference should fail
    let response = server.get("/ViewDefinition/$run").await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("GET /ViewDefinition/$run requires a ViewDefinition")
    );
}

#[tokio::test]
async fn test_run_view_definition_get_with_reference() {
    let server = common::test_server().await;

    // GET request with viewReference (not allowed per FHIR spec)
    let response = server
        .get("/ViewDefinition/$run")
        .add_query_param("viewReference", "ViewDefinition/123")
        .add_query_param("_format", "application/json")
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("GET operations cannot use complex parameters like viewReference")
    );
}

#[tokio::test]
async fn test_run_view_definition_get_with_query_params() {
    let server = common::test_server().await;

    // GET request with complex parameters should fail
    let response = server
        .get("/ViewDefinition/$run")
        .add_query_param("viewReference", "ViewDefinition/my-view")
        .add_query_param("_format", "text/csv")
        .add_query_param("header", "present")
        .add_query_param("_count", "100")
        .add_query_param("_page", "2")
        .add_query_param("_since", "2024-01-01T00:00:00Z")
        .add_query_param("patient", "Patient/123")
        .add_query_param("group", "Group/456")
        .add_query_param("source", "my-data-source")
        .await;

    // Should fail because GET cannot use complex parameters like viewReference
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    // The error should mention the first complex parameter encountered
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("viewReference")
    );
}

#[tokio::test]
async fn test_run_view_definition_get_instance_level() {
    let server = common::test_server().await;

    // Instance-level GET request
    let response = server
        .get("/ViewDefinition/my-view-def/$run")
        .add_query_param("_format", "application/json")
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("ViewDefinition lookup by ID")
    );
}

#[tokio::test]
async fn test_run_view_definition_post_with_source_parameter() {
    let server = common::test_server().await;

    // Create a request body with source parameter
    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "source",
            "valueString": "https://example.com/fhir-data"
        }, {
            "name": "viewResource",
            "resource": {
                "resourceType": "ViewDefinition",
                "status": "active",
                "resource": "Patient",
                "select": [{"column": [{"name": "id", "path": "id"}]}]
            }
        }]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("The source parameter is not yet implemented")
    );
}

#[tokio::test]
async fn test_post_viewreference_not_implemented() {
    let server = common::test_server().await;

    // Create a request body with viewReference parameter
    let request_body = json!({
        "resourceType": "Parameters",
        "parameter": [{
            "name": "viewReference",
            "valueReference": {
                "reference": "ViewDefinition/123"
            }
        }, {
            "name": "resource",
            "resource": {
                "resourceType": "Patient",
                "id": "example",
                "gender": "male"
            }
        }]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("The viewReference parameter is not yet implemented")
    );
}

#[tokio::test]
async fn test_post_group_not_implemented() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{"column": [{"name": "id", "path": "id"}]}]
                }
            },
            {
                "name": "group",
                "valueReference": {
                    "reference": "Group/test-group"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("The group parameter is not yet implemented")
    );
}

#[tokio::test]
async fn test_post_source_not_implemented() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    "select": [{"column": [{"name": "id", "path": "id"}]}]
                }
            },
            {
                "name": "source",
                "valueString": "http://example.com/fhir"
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_IMPLEMENTED);
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("The source parameter is not yet implemented")
    );
}

#[tokio::test]
async fn test_patient_filtering_incorrect_format() {
    let server = common::test_server().await;

    // This test demonstrates the issue: incorrect valueReference format
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "patient",
                "valueReference": "Patient/pt-1"  // INCORRECT: should be an object
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"},
                            {"name": "family", "path": "name.family"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-1",
                    "name": [{"family": "Cole"}]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-2",
                    "name": [{"family": "Doe"}]
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    
    // Without proper patient filter, both patients are returned
    assert!(json.is_array());
    let results = json.as_array().unwrap();
    assert_eq!(results.len(), 2, "Both patients returned when filter not parsed");
}

#[tokio::test]
async fn test_patient_filtering_correct_format() {
    let server = common::test_server().await;

    // Correct format for valueReference
    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "patient",
                "valueReference": {
                    "reference": "Patient/pt-1"  // CORRECT: object with reference property
                }
            },
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "resource": "Patient",
                    "select": [{
                        "column": [
                            {"name": "id", "path": "id"},
                            {"name": "family", "path": "name.family"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-1",
                    "name": [{"family": "Cole"}]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "pt-2",
                    "name": [{"family": "Doe"}]
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    
    // With proper patient filter, only pt-1 is returned
    assert!(json.is_array());
    let results = json.as_array().unwrap();
    assert_eq!(results.len(), 1, "Only pt-1 should be returned");
    assert_eq!(results[0]["id"], "pt-1");
    assert_eq!(results[0]["family"], "Cole");
}

#[tokio::test]
async fn test_since_parameter_in_post_body_valid() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_since",
                "valueInstant": "2023-01-01T00:00:00Z"
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
                    "id": "example"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    // Since _since filtering is not implemented, it should succeed but not filter
    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    assert!(json.is_array());
}

#[tokio::test]
async fn test_since_parameter_in_post_body_invalid() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_since",
                "valueInstant": "not-a-valid-timestamp"
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
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("_since parameter must be a valid RFC3339 timestamp")
    );
}

#[tokio::test]
async fn test_since_parameter_filtering() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_since",
                "valueInstant": "2023-06-01T00:00:00Z"
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
                            {"name": "lastUpdated", "path": "meta.lastUpdated"}
                        ]
                    }]
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "old-patient",
                    "meta": {
                        "lastUpdated": "2023-01-01T00:00:00Z"
                    }
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "new-patient",
                    "meta": {
                        "lastUpdated": "2023-12-01T00:00:00Z"
                    }
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    assert!(json.is_array());
    let results = json.as_array().unwrap();
    
    // Should only return the new patient (updated after 2023-06-01)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["id"], "new-patient");
    assert_eq!(results[0]["lastUpdated"], "2023-12-01T00:00:00Z");
}

#[tokio::test]
async fn test_since_parameter_no_meta() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_since",
                "valueInstant": "2023-06-01T00:00:00Z"
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
                    "id": "patient-without-meta"
                    // No meta field
                }
            },
            {
                "name": "resource",
                "resource": {
                    "resourceType": "Patient",
                    "id": "patient-with-meta",
                    "meta": {
                        "lastUpdated": "2023-12-01T00:00:00Z"
                    }
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();
    assert!(json.is_array());
    let results = json.as_array().unwrap();
    
    // Should only return the patient with meta.lastUpdated after _since
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["id"], "patient-with-meta");
}

#[tokio::test]
async fn test_since_parameter_wrong_value_type() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "_since",
                "valueString": "2023-01-01T00:00:00Z"  // Wrong! Should be valueInstant or valueDateTime
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
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    assert!(
        json["issue"][0]["details"]["text"]
            .as_str()
            .unwrap()
            .contains("_since parameter must use valueInstant or valueDateTime")
    );
}
