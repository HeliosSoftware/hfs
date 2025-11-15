use helios_fhir::r4::{Bundle, BundleEntry, DetectedIssue};
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_datetime_constant_debug() {
    // Create DetectedIssue resource with identified dateTime
    let detected_issue: DetectedIssue = serde_json::from_str(
        r#"{
        "resourceType": "DetectedIssue",
        "id": "di2",
        "status": "final",
        "identifiedDateTime": "2016-11-12"
    }"#,
    )
    .unwrap();

    // Create bundle manually
    let mut bundle = Bundle::default();
    bundle.r#type.value = Some("collection".to_string());

    let mut entry = BundleEntry::default();
    entry.resource = Some(helios_fhir::r4::Resource::DetectedIssue(detected_issue));
    bundle.entry = Some(vec![entry]);

    // Test with dateTime constant comparison
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "DetectedIssue",
        "constant": [
          {
            "name": "id_time",
            "valueDateTime": "2016-11-12"
          }
        ],
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "identified_value",
                "path": "identified"
            }, {
                "name": "identified_type",
                "path": "identified.type().name"
            }, {
                "name": "constant_value",
                "path": "%id_time"
            }, {
                "name": "constant_type",
                "path": "%id_time.type().name"
            }, {
                "name": "oftype_result",
                "path": "identified.ofType(dateTime)"
            }, {
                "name": "comparison",
                "path": "identified.ofType(dateTime) = %id_time"
            }]
        }]
    }"#;

    let view_definition: helios_fhir::r4::ViewDefinition =
        serde_json::from_str(view_def_json).unwrap();

    let sof_view = SofViewDefinition::R4(view_definition);
    let sof_bundle = SofBundle::R4(bundle);

    let result = run_view_definition(sof_view, sof_bundle, ContentType::Json).unwrap();
    let json_str = String::from_utf8(result).unwrap();
    println!("DateTime constant comparison debug result: {}", json_str);
}
