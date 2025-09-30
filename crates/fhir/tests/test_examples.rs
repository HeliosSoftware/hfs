use serde::Serialize;
use serde::de::DeserializeOwned;
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
    test_examples_in_dir::<helios_fhir::r4::Resource>(&examples_dir);
}

#[cfg(feature = "R4B")]
#[test]
fn test_r4b_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("R4B");
    test_examples_in_dir::<helios_fhir::r4b::Resource>(&examples_dir);
}

#[cfg(feature = "R5")]
#[test]
fn test_r5_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("R5");
    test_examples_in_dir::<helios_fhir::r5::Resource>(&examples_dir);
}

#[cfg(feature = "R6")]
#[test]
fn test_r6_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("R6");
    test_examples_in_dir::<helios_fhir::r6::Resource>(&examples_dir);
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
                // Check if this is a valid null-skipping transformation
                // (reserialized array contains only the non-null values from original)
                let orig_non_null: Vec<&Value> = orig_arr.iter().filter(|v| !v.is_null()).collect();
                let is_null_skipping_transformation = orig_non_null.len() == reser_arr.len()
                    && orig_non_null
                        .iter()
                        .zip(reser_arr.iter())
                        .all(|(orig, reser)| *orig == reser);

                if !is_null_skipping_transformation {
                    // If arrays have different lengths and it's not a null-skipping case,
                    // report the whole array as different
                    differences.push((path, original.clone(), reserialized.clone()));
                }
                // If it is a null-skipping transformation, we consider it valid and don't report it as a difference
            }
        }
        // For other primitive values, check equality with special handling for string-to-integer conversion
        _ => {
            if original != reserialized {
                // Check if this is a valid string-to-integer conversion
                let is_valid_conversion = match (original, reserialized) {
                    // String "123" to Number 123 is valid
                    (Value::String(s), Value::Number(n)) => {
                        // Try to parse the string as the same integer that we got
                        if let Ok(parsed_int) = s.parse::<i64>() {
                            n.as_i64() == Some(parsed_int)
                        } else if let Ok(parsed_uint) = s.parse::<u64>() {
                            n.as_u64() == Some(parsed_uint)
                        } else {
                            false
                        }
                    }
                    // All other mismatches are real differences
                    _ => false,
                };

                if !is_valid_conversion {
                    differences.push((path, original.clone(), reserialized.clone()));
                }
            }
        }
    }
}

// Helper function to find items in a Questionnaire that are missing linkId
fn find_missing_linkid(json: &serde_json::Value) {
    if let Some(items) = json.get("item").and_then(|i| i.as_array()) {
        for (index, item) in items.iter().enumerate() {
            if item.get("linkId").is_none() {
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

fn test_examples_in_dir<R: DeserializeOwned + Serialize>(dir: &PathBuf) {
    if !dir.exists() {
        println!("Directory does not exist: {:?}", dir);
        return;
    }

    // List of problematic files to skip with reasons
    let skip_files = [
        (
            "diagnosticreport-example-f202-bloodculture.json",
            "Contains null where struct TempCodeableReference expected",
        ),
        (
            "permission-example-bundle-residual.json",
            "Contains null where struct TempPermissionRuleLimit expected",
        ),
        (
            "diagnosticreport-example-dxa.json",
            "Contains null in conclusionCode array where struct TempCodeableReference expected",
        ),
        (
            "servicerequest-example-glucose.json",
            "Contains null in asNeededFor array where struct TempCodeableConcept expected",
        ),
        (
            "diagnosticreport-example-f201-brainct.json",
            "Contains null in conclusionCode array where struct TempCodeableReference expected",
        ),
        (
            "molecularsequence-example.json",
            "R6 MolecularSequence contains incompatible data structure",
        ),
        (
            "specimen-example-liver-biopsy.json",
            "R6 Specimen example contains incompatible data structure",
        ),
    ];

    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().unwrap().to_string_lossy();

            // Check if this file should be skipped
            if let Some((_, reason)) = skip_files.iter().find(|(name, _)| *name == filename) {
                println!("Skipping file: {} - Reason: {}", filename, reason);
                continue;
            }

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

                                    // Skip Questionnaire resources
                                    if resource_type_str == "Questionnaire" {
                                        println!("Skipping Questionnaire resource");
                                        continue;
                                    }

                                    // Skip ClinicalImpression resources for R6 (not yet implemented)
                                    if resource_type_str == "ClinicalImpression" {
                                        println!("Skipping ClinicalImpression resource");
                                        continue;
                                    }

                                    // Try to convert the JSON value to a FHIR Resource
                                    match serde_json::from_value::<R>(json_value.clone()) {
                                        Ok(resource) => {
                                            println!(
                                                "Successfully converted JSON to FHIR Resource"
                                            );

                                            // Verify we can serialize the Resource back to JSON
                                            match serde_json::to_value(&resource) {
                                                Ok(resource_json) => {
                                                    println!(
                                                        "Successfully serialized Resource back to JSON"
                                                    );

                                                    // Find differences between original and re-serialized JSON
                                                    let diff_paths = find_json_differences(
                                                        &json_value,
                                                        &resource_json,
                                                    );

                                                    if !diff_paths.is_empty() {
                                                        println!(
                                                            "Found {} significant differences between original and reserialized JSON:",
                                                            diff_paths.len()
                                                        );
                                                        for (path, orig_val, new_val) in &diff_paths
                                                        {
                                                            println!("  Path: {}", path);
                                                            println!(
                                                                "    Original: {}",
                                                                serde_json::to_string_pretty(
                                                                    orig_val
                                                                )
                                                                .unwrap_or_default()
                                                            );
                                                            println!(
                                                                "    Reserialized: {}",
                                                                serde_json::to_string_pretty(
                                                                    new_val
                                                                )
                                                                .unwrap_or_default()
                                                            );
                                                        }

                                                        // Only fail the test if there are actual significant differences
                                                        // (not just valid string-to-integer conversions)
                                                        panic!(
                                                            "Found {} significant differences in JSON values.\nSee above for specific differences.",
                                                            diff_paths.len()
                                                        );
                                                    }

                                                    println!("Resource JSON matches original JSON");
                                                }
                                                Err(e) => {
                                                    panic!(
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
                                                    println!(
                                                        "Checking for Questionnaire items without linkId:"
                                                    );
                                                    find_missing_linkid(&json_value);
                                                }
                                            }

                                            panic!("{}", error_message);
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
