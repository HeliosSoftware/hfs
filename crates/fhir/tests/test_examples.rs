use fhir::r4::Resource;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[cfg(feature = "R4")]
#[test]
fn test_r4_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("R4");
    println!("Testing R4 examples in directory: {:?}", examples_dir);
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R4B")]
#[test]
fn test_r4b_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("R4B");
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R5")]
#[test]
fn test_r5_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("R5");
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R6")]
#[test]
fn test_r6_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("R6");
    test_examples_in_dir(&examples_dir);
}

// This function is no longer needed with our simplified approach

// Function to find differences between two JSON values
fn find_json_differences(original: &Value, reserialized: &Value) -> Vec<(String, Value, Value)> {
    let mut differences = Vec::new();
    compare_json_values(original, reserialized, String::new(), &mut differences);
    differences
}

// Recursively compare JSON values and collect differences
fn compare_json_values(
    original: &Value,
    reserialized: &Value,
    path: String,
    differences: &mut Vec<(String, Value, Value)>,
) {
    match (original, reserialized) {
        (Value::Object(orig_obj), Value::Object(reser_obj)) => {
            // Check for missing keys in either direction
            let orig_keys: std::collections::HashSet<&String> = orig_obj.keys().collect();
            let reser_keys: std::collections::HashSet<&String> = reser_obj.keys().collect();

            // Keys in original but not in reserialized
            for key in orig_keys.difference(&reser_keys) {
                let new_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                differences.push((new_path, orig_obj[*key].clone(), Value::Null));
            }

            // Keys in reserialized but not in original
            for key in reser_keys.difference(&orig_keys) {
                let new_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                differences.push((new_path, Value::Null, reser_obj[*key].clone()));
            }

            // Compare values for keys that exist in both
            for key in orig_keys.intersection(&reser_keys) {
                let new_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                compare_json_values(&orig_obj[*key], &reser_obj[*key], new_path, differences);
            }
        }
        (Value::Array(orig_arr), Value::Array(reser_arr)) => {
            // Compare arrays element by element if they're the same length
            if orig_arr.len() == reser_arr.len() {
                for (i, (orig_val, reser_val)) in orig_arr.iter().zip(reser_arr.iter()).enumerate()
                {
                    let new_path = if path.is_empty() {
                        format!("[{}]", i)
                    } else {
                        format!("{}[{}]", path, i)
                    };
                    compare_json_values(orig_val, reser_val, new_path, differences);
                }
            } else {
                // If arrays have different lengths, just report the whole array as different
                differences.push((path, original.clone(), reserialized.clone()));
            }
        }
        // For other primitive values, just check equality
        _ => {
            if original != reserialized {
                differences.push((path, original.clone(), reserialized.clone()));
            }
        }
    }
}

// Helper function to find items in a Questionnaire that are missing linkId
fn find_missing_linkid(json: &serde_json::Value) {
    if let Some(items) = json.get("item").and_then(|i| i.as_array()) {
        for (index, item) in items.iter().enumerate() {
            if !item.get("linkId").is_some() {
                println!("Item at index {} is missing linkId", index);
                println!(
                    "Item content: {}",
                    serde_json::to_string_pretty(item).unwrap_or_default()
                );
            }

            // Recursively check nested items
            if let Some(nested_items) = item.get("item") {
                println!("Checking nested items for item at index {}", index);
                find_missing_linkid(&serde_json::json!({"item": nested_items}));
            }
        }
    }
}

fn test_examples_in_dir(dir: &PathBuf) {
    if !dir.exists() {
        println!("Directory does not exist: {:?}", dir);
        return;
    }

    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
            // Only process the specific file
            //if path.file_name().unwrap().to_string_lossy()
            //    != "extension-careplan-activity-title.json"
            //{
            //    continue;
            //}
            println!("Processing file: {}", path.display());

            // Read the file content
            match fs::read_to_string(&path) {
                Ok(content) => {
                    // Parse the JSON string
                    match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(json_value) => {
                            // Check if it has a resourceType field
                            if let Some(resource_type) = json_value.get("resourceType") {
                                if let Some(resource_type_str) = resource_type.as_str() {
                                    println!("Resource type: {}", resource_type_str);

                                    // Try to convert the JSON value to a FHIR Resource
                                    match serde_json::from_value::<Resource>(json_value.clone()) {
                                        Ok(resource) => {
                                            println!(
                                                "Successfully converted JSON to FHIR Resource"
                                            );

                                            // Verify we can serialize the Resource back to JSON
                                            match serde_json::to_value(&resource) {
                                                Ok(resource_json) => {
                                                    println!("Successfully serialized Resource back to JSON");

                                                    // Compare the original JSON with the re-serialized JSON
                                                    if resource_json != json_value {
                                                        // Find and report the differences
                                                        let diff_paths = find_json_differences(
                                                            &json_value,
                                                            &resource_json,
                                                        );
                                                        if !diff_paths.is_empty() {
                                                            println!("Found {} differences between original and reserialized JSON:", diff_paths.len());
                                                            for (path, orig_val, new_val) in
                                                                diff_paths
                                                            {
                                                                println!("  Path: {}", path);
                                                                println!(
                                                                    "    Original: {}",
                                                                    serde_json::to_string_pretty(
                                                                        &orig_val
                                                                    )
                                                                    .unwrap_or_default()
                                                                );
                                                                println!(
                                                                    "    Reserialized: {}",
                                                                    serde_json::to_string_pretty(
                                                                        &new_val
                                                                    )
                                                                    .unwrap_or_default()
                                                                );
                                                            }
                                                        }

                                                        // Still fail the test with assert_eq
                                                        assert_eq!(
                                                             resource_json,
                                                             json_value,
                                                             "JSON values should match.\nSee above for specific differences."
                                                         );
                                                    }

                                                    println!("Resource JSON matches original JSON");
                                                }
                                                Err(e) => {
                                                    assert!(
                                                        false,
                                                        "Error serializing Resource to JSON: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            let error_message = format!(
                                                "Error converting JSON to FHIR Resource: {}",
                                                e
                                            );
                                            println!("{}", error_message);

                                            // Try to extract more information about the missing field
                                            if error_message.contains("missing field") {
                                                // Print the JSON structure to help locate the issue
                                                println!("JSON structure:");
                                                if let Ok(pretty_json) =
                                                    serde_json::to_string_pretty(&json_value)
                                                {
                                                    println!("{}", pretty_json);
                                                }

                                                // If it's a Questionnaire, look for items without linkId
                                                if resource_type_str == "Questionnaire" {
                                                    println!("Checking for Questionnaire items without linkId:");
                                                    find_missing_linkid(&json_value);
                                                }
                                            }

                                            assert!(false, "{}", error_message);
                                        }
                                    }
                                } else {
                                    println!("resourceType is not a string");
                                }
                            } else {
                                println!("JSON does not contain a resourceType field");
                            }
                        }
                        Err(e) => {
                            println!("Error parsing JSON: {}: {}", path.display(), e);
                        }
                    }
                }
                Err(e) => {
                    println!("Error opening file: {}: {}", path.display(), e);
                }
            }
        }
    }
}
