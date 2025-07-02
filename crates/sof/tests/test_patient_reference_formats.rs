//! Tests for patient parameter handling with different reference formats
//!
//! This module tests that patient references work with both full references
//! (e.g., "Patient/pt-1") and bare IDs (e.g., "pt-1").

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

/// Helper to create test patients
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
            }
        ]
    })
}

/// Test patient parameter with full reference format (Patient/pt-1)
#[tokio::test]
async fn test_patient_parameter_with_full_reference() {
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
                "name": "patient",
                "valueString": "Patient/pt-1"
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::OK);

    let result = response.json::<serde_json::Value>();

    // Should return only patient pt-1
    assert!(result.is_array());
    let patients = result.as_array().unwrap();
    assert_eq!(patients.len(), 1);
    assert_eq!(patients[0]["id"], "pt-1");
}

/// Test patient parameter with bare ID format (pt-1)
#[tokio::test]
async fn test_patient_parameter_with_bare_id() {
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
                "name": "patient",
                "valueString": "pt-1"
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::OK);

    let result = response.json::<serde_json::Value>();

    // Should return only patient pt-1
    assert!(result.is_array(), "Expected array, got: {:?}", result);
    let patients = result.as_array().unwrap();
    assert_eq!(patients.len(), 1);
    assert_eq!(patients[0]["id"], "pt-1");
}

/// Test patient parameter with valueReference format
#[tokio::test]
async fn test_patient_parameter_with_value_reference() {
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
                "name": "patient",
                "valueReference": {
                    "reference": "Patient/pt-2"
                }
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::OK);

    let result = response.json::<serde_json::Value>();

    // Should return only patient pt-2
    assert!(result.is_array());
    let patients = result.as_array().unwrap();
    assert_eq!(patients.len(), 1);
    assert_eq!(patients[0]["id"], "pt-2");
}

/// Test filtering observations by patient with bare ID
#[tokio::test]
async fn test_observation_filtering_with_bare_patient_id() {
    let server = common::test_server().await;

    let view_def = json!({
        "resourceType": "ViewDefinition",
        "id": "observation-view",
        "status": "active",
        "resource": "Observation",
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "code",
                "path": "code.coding[0].code"
            }]
        }]
    });

    let bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Observation",
                    "id": "obs-1",
                    "status": "final",
                    "code": {
                        "coding": [{
                            "system": "http://loinc.org",
                            "code": "8302-2"
                        }]
                    },
                    "subject": {
                        "reference": "Patient/pt-1"
                    }
                }
            },
            {
                "resource": {
                    "resourceType": "Observation",
                    "id": "obs-2",
                    "status": "final",
                    "code": {
                        "coding": [{
                            "system": "http://loinc.org",
                            "code": "8310-5"
                        }]
                    },
                    "subject": {
                        "reference": "Patient/pt-2"
                    }
                }
            }
        ]
    });

    let parameters = json!({
        "resourceType": "Parameters",
        "parameter": [
            {
                "name": "viewResource",
                "resource": view_def
            },
            {
                "name": "resource",
                "resource": bundle
            },
            {
                "name": "patient",
                "valueString": "pt-1"  // Using bare ID
            }
        ]
    });

    let response = server
        .post("/ViewDefinition/$run")
        .content_type("application/json")
        .json(&parameters)
        .await;

    response.assert_status(StatusCode::OK);

    let result = response.json::<serde_json::Value>();

    // Should return only observation for patient pt-1
    assert!(result.is_array());
    let observations = result.as_array().unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0]["id"], "obs-1");
}
