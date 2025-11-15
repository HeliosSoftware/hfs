use helios_serde::json;
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_boolean_constant_debug() {
    // Create test patient with deceasedBoolean
    let patient_json = r#"{
        "resourceType": "Patient",
        "id": "pt2",
        "deceasedBoolean": true
    }"#;

    // First test: Check if deceased field is accessible
    let view_def_json1 = r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "deceased_exists",
                "path": "deceased.exists()"
            }, {
                "name": "deceased_oftype_exists",
                "path": "deceased.ofType(boolean).exists()"
            }, {
                "name": "deceased_value",
                "path": "deceased"
            }, {
                "name": "deceasedBoolean_direct",
                "path": "deceasedBoolean"
            }]
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

    println!("=== Test 1: Checking deceased field access ===");
    let view_definition: helios_fhir::r4::ViewDefinition = json::from_str(view_def_json1).unwrap();
    let bundle: helios_fhir::r4::Bundle = json::from_str(&bundle_json).unwrap();

    let sof_view = SofViewDefinition::R4(view_definition);
    let sof_bundle = SofBundle::R4(bundle.clone());

    let result = run_view_definition(sof_view, sof_bundle, ContentType::Json).unwrap();
    let json_str = String::from_utf8(result).unwrap();
    println!("Result 1: {}", json_str);

    // Second test: With constant
    let view_def_json2 = r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "constant": [{
            "name": "is_deceased",
            "valueBoolean": true
        }],
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "where_result",
                "path": "deceased.ofType(boolean).exists() and deceased.ofType(boolean) = %is_deceased"
            }]
        }]
    }"#;

    println!("\n=== Test 2: With constant comparison ===");
    let view_definition2: helios_fhir::r4::ViewDefinition = json::from_str(view_def_json2).unwrap();
    let bundle2: helios_fhir::r4::Bundle = json::from_str(&bundle_json).unwrap();

    let sof_view2 = SofViewDefinition::R4(view_definition2);
    let sof_bundle2 = SofBundle::R4(bundle2);

    let result2 = run_view_definition(sof_view2, sof_bundle2, ContentType::Json).unwrap();
    let json_str2 = String::from_utf8(result2).unwrap();
    println!("Result 2: {}", json_str2);
}
