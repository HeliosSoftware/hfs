use helios_fhir::r4::{Bundle, BundleEntry, Patient};
use helios_serde::json;
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_boolean_constant_debug2() {
    // Create patient resource directly
    let patient: Patient = json::from_str(
        r#"{
        "resourceType": "Patient",
        "id": "pt2",
        "deceasedBoolean": true
    }"#,
    )
    .unwrap();

    println!("Patient deceased field: {:?}", patient.deceased);

    // Create bundle manually
    let mut bundle = Bundle::default();
    bundle.r#type.value = Some("collection".to_string());

    let mut entry = BundleEntry::default();
    entry.resource = Some(helios_fhir::r4::Resource::Patient(patient));
    bundle.entry = Some(vec![entry]);

    // Test ViewDefinition that should show all fields
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "resource_type",
                "path": "resourceType"
            }]
        }]
    }"#;

    let view_definition: helios_fhir::r4::ViewDefinition = json::from_str(view_def_json).unwrap();

    let sof_view = SofViewDefinition::R4(view_definition);
    let sof_bundle = SofBundle::R4(bundle);

    let result = run_view_definition(sof_view, sof_bundle, ContentType::Json).unwrap();
    let json_str = String::from_utf8(result).unwrap();
    println!("Basic test result: {}", json_str);
}
