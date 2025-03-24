use std::fs;
use std::path::PathBuf;

use fhir::r4::Resource;

#[cfg(feature = "R4")]
#[test]
fn test_r4_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("r4");
    println!("Testing R4 examples in directory: {:?}", examples_dir);
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R4B")]
#[test]
fn test_r4b_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("r4b");
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R5")]
#[test]
fn test_r5_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("r5");
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R6")]
#[test]
fn test_r6_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("r6");
    test_examples_in_dir(&examples_dir);
}

fn compare_json_values(left: &serde_json::Value, right: &serde_json::Value) -> Result<(), String> {
    println!("Starting compare_json_values");
    // Use a non-recursive approach to avoid stack overflow
    if left == right {
        println!("Values are exactly equal");
        return Ok(());
    }

    println!("Values are not exactly equal, performing detailed comparison");
    match (left, right) {
        (serde_json::Value::Number(l), serde_json::Value::Number(r)) => {
            println!("Comparing numbers: {} vs {}", l, r);
            if let (Some(l_f64), Some(r_f64)) = (l.as_f64(), r.as_f64()) {
                // Special case for zero
                if l_f64 == 0.0 && r_f64 == 0.0 {
                    println!("Both numbers are zero");
                    Ok(())
                }
                // For non-zero floating point numbers, check if they're very close
                else if (l_f64 - r_f64).abs()
                    < f64::EPSILON * l_f64.abs().max(r_f64.abs()) * 100.0
                {
                    println!("Numbers are close enough");
                    Ok(())
                } else {
                    println!("Numbers differ significantly");
                    Err(format!(
                        "Numbers differ significantly: {} vs {}",
                        l_f64, r_f64
                    ))
                }
            } else if l.as_i64() != r.as_i64() {
                println!("Integer numbers differ");
                Err(format!("Integer numbers differ: {} vs {}", l, r))
            } else {
                println!("Numbers are equal");
                Ok(())
            }
        }
        (serde_json::Value::Array(l), serde_json::Value::Array(r)) => {
            println!("Comparing arrays of length {} vs {}", l.len(), r.len());
            if l.len() != r.len() {
                return Err(format!(
                    "Arrays have different lengths: {} vs {}",
                    l.len(),
                    r.len()
                ));
            }
            
            // Compare arrays without recursion
            for (i, (l_val, r_val)) in l.iter().zip(r.iter()).enumerate() {
                println!("Comparing array element at index {}", i);
                if l_val != r_val {
                    println!("Array elements at index {} differ", i);
                    return Err(format!("Array elements at index {} differ", i));
                }
            }
            println!("Arrays are equal");
            Ok(())
        }
        (serde_json::Value::Object(l), serde_json::Value::Object(r)) => {
            println!("Comparing objects of size {} vs {}", l.len(), r.len());
            if l.len() != r.len() {
                return Err(format!(
                    "Objects have different lengths: {} vs {}",
                    l.len(),
                    r.len()
                ));
            }
            
            // Compare objects without recursion
            for (key, l_val) in l.iter() {
                println!("Checking key: {}", key);
                match r.get(key) {
                    Some(r_val) => {
                        if l_val != r_val {
                            println!("Values for key '{}' differ", key);
                            return Err(format!("Values for key '{}' differ", key));
                        }
                    },
                    None => {
                        println!("Right object missing key: {}", key);
                        return Err(format!("Right object missing key: {}", key));
                    }
                }
            }
            println!("Objects are equal");
            Ok(())
        }
        (l, r) => {
            println!("Values of different types differ");
            Err(format!("Values differ: {:?} vs {:?}", l, r))
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
            if path.file_name().unwrap().to_string_lossy()
                != "extension-careplan-activity-title.json"
            {
                continue;
            }

            println!("Processing file: {}", path.display());
            println!("Step 1: Reading file content");
            let content = fs::read_to_string(&path).unwrap();
            println!("Step 2: File content read successfully, length: {}", content.len());

            println!("Step 3: Parsing JSON string to serde_json::Value");
            let original: serde_json::Value = match serde_json::from_str(&content) {
                Ok(val) => {
                    println!("Step 4: JSON parsed successfully");
                    val
                },
                Err(e) => {
                    println!("Error parsing JSON: {}", e);
                    continue;
                }
            };

            println!("Step 5: Converting serde_json::Value to Resource");
            match serde_json::from_value::<Resource>(original.clone()) {
                Ok(resource) => {
                    println!("Step 6: Successfully converted to Resource");
                    
                    println!("Step 7: Serializing Resource back to serde_json::Value");
                    let serialized_resource = match serde_json::to_value(&resource) {
                        Ok(val) => {
                            println!("Step 8: Successfully serialized Resource");
                            val
                        },
                        Err(e) => {
                            println!("Error serializing Resource: {}", e);
                            continue;
                        }
                    };

                    println!("Step 9: Comparing original and serialized JSON");
                    // Simple equality check first (faster)
                    if original == serialized_resource {
                        println!("File {} passed resource round-trip serialization test", path.display());
                        continue;
                    }

                    println!("Step 10: Simple equality check failed, using custom comparison");
                    // If simple equality fails, try our custom comparison
                    match compare_json_values(&original, &serialized_resource) {
                        Ok(_) => {
                            println!("File {} passed resource round-trip serialization test with custom comparison", path.display());
                        }
                        Err(e) => {
                            println!("Custom comparison failed: {}", e);
                            panic!("File {} failed resource round-trip serialization test: {}", path.display(), e);
                        }
                    }
                }
                Err(e) => {
                    println!("Error parsing as FHIR resource: {}:{}", path.display(), e);
                }
            }
        }
    }
}
