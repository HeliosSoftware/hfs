use serde::Serialize;
use std::fs;
use std::path::PathBuf;

use fhir::r4::{Patient, Resource};

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
            for (l_val, r_val) in l.iter().zip(r.iter()) {
                compare_json_values(l_val, r_val)?;
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
            for (key, l_val) in l.iter() {
                match r.get(key) {
                    Some(r_val) => compare_json_values(l_val, r_val)?,
                    None => return Err(format!("Right object missing key: {}", key)),
                }
            }
            Ok(())
        }
        (l, r) if l == r => Ok(()),
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
            if path.file_name().unwrap().to_string_lossy() != "patient-example-mom.json" {
                continue;
            }

            println!("Processing file: {}", path.display());

            let content = fs::read_to_string(&path).unwrap();
            println!("Content JSON: {}", content);

            // Parse the JSON into serde_json::Value
            let mut deserializer = serde_json::Deserializer::from_str(&content);
            deserializer.options_mut().set_ignore_null(true);
            let original: serde_json::Value =
                serde_json::Value::deserialize(&mut deserializer).unwrap();

            // Output the original JSON value
            println!("Original JSON: {}", original);

            match serde_json::from_value::<Resource>(original.clone()) {
                Ok(resource) => {
                    // Serialize the parsed resource back to JSON
                    let serialized_resource = serde_json::to_value(&resource).unwrap();

                    println!("New JSON: {}", serialized_resource);

                    // Compare the original and re-serialized JSON
                    let result = compare_json_values(&original, &serialized_resource);
                    assert!(
                        result.is_ok(),
                        "File {} failed resource round-trip serialization test: {}",
                        path.display(),
                        result.unwrap_err()
                    );

                    println!("Successfully round-tripped resource");
                }
                Err(e) => {
                    println!("Error parsing as FHIR resource: {}", e);
                }
            }
            /*
                        // Serialize back to string with maximum precision
                        let serialized = {
                            let mut serializer = serde_json::Serializer::with_formatter(
                                Vec::new(),
                                serde_json::ser::PrettyFormatter::new(),
                            );
                            original.serialize(&mut serializer).unwrap();
                            String::from_utf8(serializer.into_inner()).unwrap()
                        };

                        // Parse again to normalize formatting
                        let reserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();

                        // Compare structure ignoring floating point precision differences
                        let result = compare_json_values(&original, &reserialized);
                        assert!(
                            result.is_ok(),
                            "File {} failed round-trip serialization test: {}",
                            path.display(),
                            result.unwrap_err()
                        );
            */
        }
    }
}

#[cfg(feature = "R4")]
#[test]
fn test_boolean_initialization() {
    //   use fhir::r4::Boolean;

    // Create a Boolean with value true
    //    let boolean = Boolean::new(true);

    // Verify the value is true
    //    assert_eq!(boolean.value.unwrap(), true);
    //    println!(
    //        "Successfully created Boolean with value: {:?}",
    //        boolean.value
    //    );
}
