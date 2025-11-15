use helios_fhir::r4::{Bundle, BundleEntry, Observation};
use helios_serde::json;
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_instant_constant_debug() {
    // Create Observation resource with effectiveInstant
    let observation: Observation = json::from_str(
        r#"{
        "resourceType": "Observation",
        "id": "o1",
        "status": "final",
        "code": { "text": "code" },
        "effectiveInstant": "2015-02-07T13:28:17.239+02:00"
    }"#,
    )
    .unwrap();

    // Create bundle manually
    let mut bundle = Bundle::default();
    bundle.r#type.value = Some("collection".to_string());

    let mut entry = BundleEntry::default();
    entry.resource = Some(helios_fhir::r4::Resource::Observation(observation));
    bundle.entry = Some(vec![entry]);

    // Test with instant constant comparison
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Observation",
        "constant": [
          {
            "name": "eff",
            "valueInstant": "2015-02-07T13:28:17.239+02:00"
          }
        ],
        "select": [{
            "column": [{
                "name": "id",
                "path": "id"
            }, {
                "name": "effective_value",
                "path": "effective"
            }, {
                "name": "effective_type",
                "path": "effective.type().name"
            }, {
                "name": "effective_instant",
                "path": "effective.ofType(instant)"
            }, {
                "name": "effective_instant_value",
                "path": "effective.ofType(instant).value"
            }, {
                "name": "constant_value",
                "path": "%eff"
            }, {
                "name": "constant_value_value",
                "path": "%eff.value"
            }, {
                "name": "constant_type",
                "path": "%eff.type().name"
            }, {
                "name": "comparison",
                "path": "effective.ofType(instant) = %eff"
            }, {
                "name": "direct_comparison",
                "path": "effectiveInstant = %eff"
            }]
        }]
    }"#;

    let view_definition: helios_fhir::r4::ViewDefinition = json::from_str(view_def_json).unwrap();

    let sof_view = SofViewDefinition::R4(view_definition);
    let sof_bundle = SofBundle::R4(bundle);

    let result = run_view_definition(sof_view, sof_bundle, ContentType::Json).unwrap();
    let json_str = String::from_utf8(result).unwrap();
    println!("Instant constant comparison debug result: {}", json_str);
}
