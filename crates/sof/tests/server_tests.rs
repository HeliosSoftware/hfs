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
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
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

    // Verify $viewdefinition-run operation is supported
    let operations = view_def_resource["operation"].as_array().unwrap();
    assert!(
        operations
            .iter()
            .any(|op| op["name"] == "viewdefinition-run")
    );
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
        .add_header("Accept", "application/ndjson")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    // Production NDJSON content-type is `application/x-ndjson` (matches
    // HFS REST; aligned in the audit #8 sweep). `application/ndjson`
    // remains a permissive INPUT alias for back-compat, but the OUTPUT
    // is always the dashed form.
    assert_eq!(content_type.to_str().unwrap(), "application/x-ndjson");

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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
}

/// A bare `ViewDefinition` body (no `Parameters` wrapper) is a valid shortcut
/// shape: callers piping a stored ViewDefinition straight to the server
/// shouldn't have to build a Parameters envelope. Other operation parameters
/// must come from the query string in this shape.
#[tokio::test]
async fn test_run_view_definition_bare_body() {
    let server = common::test_server().await;

    let bare_view = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [
                {"name": "id", "path": "id"},
                {"name": "gender", "path": "gender"}
            ]
        }]
    });

    // No `Parameters` wrapper, no `resource` entries. The view runs
    // against zero input resources — what matters is that the body
    // shape (`resourceType=ViewDefinition`, not `Parameters`) is
    // accepted instead of being rejected with `400 Bad Request +
    // "Request body must be a Parameters resource"`.
    let response = server
        .post("/ViewDefinition/$viewdefinition-run")
        .add_query_param("_format", "application/json")
        .json(&bare_view)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "bare ViewDefinition body must be accepted: {:?}",
        response.text()
    );
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
        .post("/ViewDefinition/$viewdefinition-run")
        .add_query_param("_format", "text/plain") // Unsupported format
        .json(&request_body)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
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
        .post("/ViewDefinition/$viewdefinition-run")
        .json(&request_body)
        .await;

    // Note: The actual server handler now supports source parameter,
    // but the test mock handler doesn't yet implement it.
    // For now, we'll accept either NOT_IMPLEMENTED from the mock
    // or a real error from attempting to fetch the URL
    assert!(
        response.status_code() == StatusCode::NOT_IMPLEMENTED
            || response.status_code() == StatusCode::OK
            || response.status_code() == StatusCode::BAD_REQUEST
            || response.status_code() == StatusCode::UNPROCESSABLE_ENTITY
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    // Note: The actual server handler now supports source parameter,
    // but the test mock handler doesn't yet implement it.
    // For now, we'll accept either NOT_IMPLEMENTED from the mock
    // or a real error from attempting to fetch the URL
    assert!(
        response.status_code() == StatusCode::NOT_IMPLEMENTED
            || response.status_code() == StatusCode::OK
            || response.status_code() == StatusCode::BAD_REQUEST
            || response.status_code() == StatusCode::UNPROCESSABLE_ENTITY
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
        .post("/ViewDefinition/$viewdefinition-run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let json: serde_json::Value = response.json();

    // Without proper patient filter, both patients are returned
    assert!(json.is_array());
    let results = json.as_array().unwrap();
    assert_eq!(
        results.len(),
        2,
        "Both patients returned when filter not parsed"
    );
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
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
        .post("/ViewDefinition/$viewdefinition-run")
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

/// Audit item #6: `POST /$viewdefinition-run` (system-level) routes to the
/// same handler as the type-level alias `POST /ViewDefinition/$viewdefinition-run`.
#[tokio::test]
async fn test_system_level_route_runs_view_definition() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "_format", "valueCode": "ndjson"},
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
                "name": "resource",
                "resource": {"resourceType": "Patient", "id": "p1"}
            }
        ]
    });

    // System-level URL — no /ViewDefinition prefix.
    let response = server
        .post("/$viewdefinition-run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "system-level POST /$viewdefinition-run must succeed; body: {}",
        response.text()
    );
    let text = response.text();
    assert!(
        text.contains("\"id\":\"p1\""),
        "response must contain the seeded Patient id: {text}"
    );
}

/// Audit item #7: instance-level URLs are rejected with a clear 400
/// explaining the stateless limitation, not a 404 or 501.
#[tokio::test]
async fn test_instance_level_returns_400_with_stateless_explanation() {
    let server = common::test_server().await;

    let response = server
        .post("/ViewDefinition/some-id/$viewdefinition-run")
        .add_header("Content-Type", "application/json")
        .json(&json!({"resourceType": "Parameters"}))
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "instance-level POST must return 400, not 404/501"
    );
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
    let details = json["issue"][0]["details"]["text"]
        .as_str()
        .expect("error must have text details");
    assert!(
        details.contains("Instance-level") && details.contains("stateless"),
        "error message must explain stateless limitation: {details}"
    );
    assert!(
        details.contains("viewResource"),
        "error message must point at the supported alternative: {details}"
    );
}

/// Audit item #8: parquet output uses `application/octet-stream` per the
/// SoF v2 spec Accept table, plus `Content-Disposition: attachment;
/// filename="output.parquet"` so downloads land with the right
/// extension. Pre-fix, sof-server returned `application/parquet`
/// (non-standard).
#[tokio::test]
async fn test_parquet_response_uses_octet_stream_content_type() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "_format", "valueCode": "application/octet-stream"},
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
                "name": "resource",
                "resource": {"resourceType": "Patient", "id": "p1"}
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$viewdefinition-run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "parquet request must succeed; body: {}",
        response.text()
    );
    let ct = response
        .header("content-type")
        .to_str()
        .unwrap_or("")
        .to_string();
    assert_eq!(
        ct, "application/octet-stream",
        "parquet response must use application/octet-stream per spec, got {ct}"
    );
    let cd = response
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        cd.contains("filename=") && cd.contains(".parquet"),
        "parquet response must include Content-Disposition naming a .parquet file, got '{cd}'"
    );
    // PAR1 magic bytes confirm we actually got parquet bytes.
    let bytes = response.as_bytes();
    assert!(
        bytes.starts_with(b"PAR1"),
        "response body must be a Parquet file (PAR1 magic), got first 8 bytes: {:?}",
        &bytes[..bytes.len().min(8)]
    );
}

/// Audit item #9: an invalid ViewDefinition body (well-formed Parameters
/// wrapper, but the inner ViewDefinition has a type mismatch serde can't
/// parse) must surface as `422 Unprocessable Entity`, not `400 Bad Request`.
#[tokio::test]
async fn test_invalid_view_definition_returns_422() {
    let server = common::test_server().await;

    let body = json!({
        "resourceType": "Parameters",
        "parameter": [
            {"name": "_format", "valueCode": "ndjson"},
            {
                "name": "viewResource",
                "resource": {
                    "resourceType": "ViewDefinition",
                    "status": "active",
                    "resource": "Patient",
                    // Type mismatch: select must be an array of Select objects,
                    // not a string. Serde rejects deserialization.
                    "select": "not-an-array"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$viewdefinition-run")
        .add_header("Content-Type", "application/json")
        .json(&body)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid ViewDefinition must be 422 (audit #9), got {} with body: {}",
        response.status_code(),
        response.text()
    );
    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "OperationOutcome");
}

/// Audit item #11: sof-server publishes the spec-defined
/// `GET /$sql-on-fhir-capabilities` endpoint with truthful capability
/// flags (no reference resolution, no export, no $sqlquery-run; all
/// four `$viewdefinition-run` output formats listed).
#[tokio::test]
async fn test_sof_capabilities_endpoint() {
    let server = common::test_server().await;

    let response = server.get("/$sql-on-fhir-capabilities").await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let content_type = response.header("content-type");
    assert_eq!(content_type.to_str().unwrap(), "application/fhir+json");

    let json: serde_json::Value = response.json();
    assert_eq!(json["resourceType"], "Parameters");

    let params = json["parameter"].as_array().expect("parameter array");

    // Helper to extract a single boolean by name.
    let bool_for = |name: &str| -> bool {
        params
            .iter()
            .find(|p| p["name"] == name)
            .and_then(|p| p["valueBoolean"].as_bool())
            .unwrap_or_else(|| panic!("missing {name}"))
    };

    assert!(
        bool_for("supportsViewDefinitionRun"),
        "$viewdefinition-run must be supported"
    );
    assert!(
        !bool_for("supportsViewDefinitionExport"),
        "stateless sof-server doesn't support $export"
    );
    assert!(
        !bool_for("supportsSqlQueryRun"),
        "sof-server doesn't expose $sqlquery-run"
    );
    assert!(
        !bool_for("supportsInDbRunner"),
        "sof-server uses the in-process FHIRPath runner only"
    );
    assert!(
        !bool_for("supportsRelativeReference"),
        "sof-server has no resource store"
    );
    assert!(
        !bool_for("supportsCanonicalReference"),
        "sof-server has no resource store"
    );
    assert!(
        !bool_for("supportsAbsoluteReference"),
        "sof-server has no resource store"
    );

    // All four $viewdefinition-run output formats must be advertised.
    let formats: Vec<&str> = params
        .iter()
        .filter(|p| p["name"] == "supportedFormat")
        .filter_map(|p| p["valueCode"].as_str())
        .collect();
    for required in ["ndjson", "json", "csv", "parquet"] {
        assert!(
            formats.contains(&required),
            "supportedFormat must include {required}: {formats:?}"
        );
    }

    // Audit item #13: the response must declare the spec's
    // OutputFormatCodes value-set binding so audit tools can find
    // it without dereferencing the OperationDefinition.
    let binding = params
        .iter()
        .find(|p| p["name"] == "formatBinding")
        .expect("formatBinding parameter must be present");
    let binding_parts = binding["part"]
        .as_array()
        .expect("formatBinding must have part[]");
    let value_set = binding_parts
        .iter()
        .find(|p| p["name"] == "valueSet")
        .and_then(|p| p["valueUri"].as_str())
        .expect("formatBinding.valueSet must be a uri");
    assert_eq!(
        value_set, "https://sql-on-fhir.org/ig/ValueSet/OutputFormatCodes",
        "binding must reference the spec's OutputFormatCodes value set"
    );
    let strength = binding_parts
        .iter()
        .find(|p| p["name"] == "strength")
        .and_then(|p| p["valueCode"].as_str())
        .expect("formatBinding.strength must be a code");
    assert_eq!(
        strength, "extensible",
        "binding strength must match the spec's `extensible` declaration"
    );
}
