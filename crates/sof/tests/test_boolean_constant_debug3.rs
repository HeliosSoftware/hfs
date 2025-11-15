use helios_fhir::r4::{Bundle, BundleEntry, Patient};
use helios_serde::json;
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_boolean_constant_debug3() {
    // Create patient resource directly
    let patient: Patient = json::from_str(
        r#"{
        "resourceType": "Patient",
        "id": "pt2",
        "deceasedBoolean": true
    }"#,
    )
    .unwrap();

    // Create bundle manually
    let mut bundle = Bundle::default();
    bundle.r#type.value = Some("collection".to_string());

    let mut entry = BundleEntry::default();
    entry.resource = Some(helios_fhir::r4::Resource::Patient(patient));
    bundle.entry = Some(vec![entry]);

    // Now test with deceased field access
    let view_def_json = r#"{
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
                "name": "deceased_value",
                "path": "deceased"
            }, {
                "name": "deceased_oftype_bool",
                "path": "deceased.ofType(boolean)"
            }, {
                "name": "deceased_oftype_bool_exists",
                "path": "deceased.ofType(boolean).exists()"
            }]
        }]
    }"#;

    let view_definition: helios_fhir::r4::ViewDefinition = json::from_str(view_def_json).unwrap();

    let sof_view = SofViewDefinition::R4(view_definition);
    let sof_bundle = SofBundle::R4(bundle);

    let result = run_view_definition(sof_view, sof_bundle, ContentType::Json).unwrap();
    let json_str = String::from_utf8(result).unwrap();
    println!("Deceased field access test result: {}", json_str);
}
