use helios_fhir::r4::{Bundle, ViewDefinition};
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_where_clause_boolean_validation() {
    // Test 1: Valid boolean where clause (comparison)
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id"
                    }
                ]
            }
        ],
        "where": [
            {
                "path": "active = true"
            }
        ]
    }"#;

    let view_def: ViewDefinition = serde_json::from_str(view_def_json).unwrap();
    let bundle = create_test_bundle();

    let result = run_view_definition(
        SofViewDefinition::R4(view_def),
        SofBundle::R4(bundle),
        ContentType::Json,
    );

    assert!(
        result.is_ok(),
        "Boolean comparison where clause should succeed"
    );

    // Test 2: Valid boolean where clause (exists)
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id"
                    }
                ]
            }
        ],
        "where": [
            {
                "path": "name.exists()"
            }
        ]
    }"#;

    let view_def: ViewDefinition = serde_json::from_str(view_def_json).unwrap();
    let bundle = create_test_bundle();

    let result = run_view_definition(
        SofViewDefinition::R4(view_def),
        SofBundle::R4(bundle),
        ContentType::Json,
    );

    assert!(result.is_ok(), "exists() where clause should succeed");

    // Test 3: Invalid non-boolean where clause (string property)
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id"
                    }
                ]
            }
        ],
        "where": [
            {
                "path": "name.family"
            }
        ]
    }"#;

    let view_def: ViewDefinition = serde_json::from_str(view_def_json).unwrap();
    let bundle = create_test_bundle();

    let result = run_view_definition(
        SofViewDefinition::R4(view_def),
        SofBundle::R4(bundle),
        ContentType::Json,
    );

    assert!(result.is_err(), "String property where clause should fail");
    if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
            error_msg.contains("cannot be used as a boolean condition"),
            "Error should mention boolean condition requirement, got: {}",
            error_msg
        );
    }

    // Test 4: Valid collection where clause
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id"
                    }
                ]
            }
        ],
        "where": [
            {
                "path": "name"
            }
        ]
    }"#;

    let view_def: ViewDefinition = serde_json::from_str(view_def_json).unwrap();
    let bundle = create_test_bundle();

    let result = run_view_definition(
        SofViewDefinition::R4(view_def),
        SofBundle::R4(bundle),
        ContentType::Json,
    );

    assert!(
        result.is_ok(),
        "Collection where clause should succeed (checks for non-empty)"
    );
}

fn create_test_bundle() -> Bundle {
    let patient_json = r#"{
        "resourceType": "Patient",
        "id": "test-patient",
        "active": true,
        "name": [{
            "family": "Smith",
            "given": ["John"]
        }]
    }"#;

    let bundle_json = format!(
        r#"{{
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [{{
            "resource": {}
        }}]
    }}"#,
        patient_json
    );

    serde_json::from_str(&bundle_json).unwrap()
}
