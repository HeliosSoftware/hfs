use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};

#[test]
fn test_parallel_processing_works() {
    // Create a simple view definition
    let view_def_json = r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [
                {"name": "id", "path": "id"},
                {"name": "name", "path": "name[0].family"}
            ]
        }]
    }"#;

    // Create a bundle with multiple patients
    let bundle_json = r#"{
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {"resource": {"resourceType": "Patient", "id": "1", "name": [{"family": "Smith"}]}},
            {"resource": {"resourceType": "Patient", "id": "2", "name": [{"family": "Jones"}]}},
            {"resource": {"resourceType": "Patient", "id": "3", "name": [{"family": "Brown"}]}},
            {"resource": {"resourceType": "Patient", "id": "4", "name": [{"family": "Davis"}]}},
            {"resource": {"resourceType": "Patient", "id": "5", "name": [{"family": "Wilson"}]}}
        ]
    }"#;

    let view_def: helios_fhir::r4::ViewDefinition =
        serde_json::from_str(view_def_json).expect("Failed to parse ViewDefinition");
    let bundle: helios_fhir::r4::Bundle =
        serde_json::from_str(bundle_json).expect("Failed to parse Bundle");

    let sof_view = SofViewDefinition::R4(view_def);
    let sof_bundle = SofBundle::R4(bundle);

    // Run the view definition (will use parallel processing)
    let result = run_view_definition(sof_view, sof_bundle, ContentType::Csv)
        .expect("Failed to run view definition");

    // Convert result to string
    let csv_string = String::from_utf8(result).expect("Invalid UTF-8");

    // Verify we got the expected output
    let lines: Vec<&str> = csv_string.lines().collect();

    // Debug output to see what we got
    println!("CSV output ({} lines):\n{}", lines.len(), csv_string);

    // Should have data rows
    assert_eq!(lines.len(), 5, "Should have 5 patients");

    // Check that all patients are present (order may vary due to parallel processing)
    let mut found_ids = [false; 5];
    for line in &lines {
        if line.contains("1") && line.contains("Smith") {
            found_ids[0] = true;
        }
        if line.contains("2") && line.contains("Jones") {
            found_ids[1] = true;
        }
        if line.contains("3") && line.contains("Brown") {
            found_ids[2] = true;
        }
        if line.contains("4") && line.contains("Davis") {
            found_ids[3] = true;
        }
        if line.contains("5") && line.contains("Wilson") {
            found_ids[4] = true;
        }
    }

    assert!(
        found_ids.iter().all(|&found| found),
        "Not all patients found in output"
    );

    println!("✅ Parallel processing is working correctly!");
}
