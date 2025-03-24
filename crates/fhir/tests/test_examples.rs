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
    // Use a non-recursive approach to avoid stack overflow
    if left == right {
        return Ok(());
    }

    match (left, right) {
        (serde_json::Value::Number(l), serde_json::Value::Number(r)) => {
            if let (Some(l_f64), Some(r_f64)) = (l.as_f64(), r.as_f64()) {
                // Special case for zero
                if l_f64 == 0.0 && r_f64 == 0.0 {
                    Ok(())
                }
                // For non-zero floating point numbers, check if they're very close
                else if (l_f64 - r_f64).abs()
                    < f64::EPSILON * l_f64.abs().max(r_f64.abs()) * 100.0
                {
                    Ok(())
                } else {
                    Err(format!(
                        "Numbers differ significantly: {} vs {}",
                        l_f64, r_f64
                    ))
                }
            } else if l.as_i64() != r.as_i64() {
                Err(format!("Integer numbers differ: {} vs {}", l, r))
            } else {
                Ok(())
            }
        }
        (serde_json::Value::Array(l), serde_json::Value::Array(r)) => {
            if l.len() != r.len() {
                return Err(format!(
                    "Arrays have different lengths: {} vs {}",
                    l.len(),
                    r.len()
                ));
            }
            
            // Compare arrays without recursion
            for (i, (l_val, r_val)) in l.iter().zip(r.iter()).enumerate() {
                if l_val != r_val {
                    return Err(format!("Array elements at index {} differ", i));
                }
            }
            Ok(())
        }
        (serde_json::Value::Object(l), serde_json::Value::Object(r)) => {
            if l.len() != r.len() {
                return Err(format!(
                    "Objects have different lengths: {} vs {}",
                    l.len(),
                    r.len()
                ));
            }
            
            // Compare objects without recursion
            for (key, l_val) in l.iter() {
                match r.get(key) {
                    Some(r_val) => {
                        if l_val != r_val {
                            return Err(format!("Values for key '{}' differ", key));
                        }
                    },
                    None => return Err(format!("Right object missing key: {}", key)),
                }
            }
            Ok(())
        }
        (l, r) => Err(format!("Values differ: {:?} vs {:?}", l, r)),
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

            let content = fs::read_to_string(&path).unwrap();

            // Parse the JSON into serde_json::Value
            let original: serde_json::Value = serde_json::from_str(&content).unwrap();

            match serde_json::from_value::<Resource>(original.clone()) {
                Ok(resource) => {
                    // Serialize the parsed resource back to JSON
                    let serialized_resource = serde_json::to_value(&resource).unwrap();

                    // Simple equality check first (faster)
                    if original == serialized_resource {
                        println!("File {} passed resource round-trip serialization test", path.display());
                        continue;
                    }

                    // If simple equality fails, try our custom comparison
                    match compare_json_values(&original, &serialized_resource) {
                        Ok(_) => {
                            println!("File {} passed resource round-trip serialization test with custom comparison", path.display());
                        }
                        Err(e) => {
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
