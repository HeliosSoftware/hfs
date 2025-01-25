use std::fs;
use std::path::PathBuf;

#[cfg(feature = "R4")]
#[test]
fn test_r4_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test")
        .join("data")
        .join("r4");
    println!("Testing R4 examples in directory: {:?}", examples_dir);
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R4B")]
#[test]
fn test_r4b_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test")
        .join("data")
        .join("r4b");
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R5")]
#[test]
fn test_r5_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test")
        .join("data")
        .join("r5");
    test_examples_in_dir(&examples_dir);
}

#[cfg(feature = "R6")]
#[test]
fn test_r6_examples() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("test")
        .join("data")
        .join("r6");
    test_examples_in_dir(&examples_dir);
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
            println!("Testing file: {}", path.display());
            let content = fs::read_to_string(&path).unwrap();

            // Parse the JSON into serde_json::Value
            let original: serde_json::Value = serde_json::from_str(&content).unwrap();

            // Serialize back to string
            let serialized = serde_json::to_string_pretty(&original).unwrap();

            // Parse again to normalize formatting
            let reserialized: serde_json::Value = serde_json::from_str(&serialized).unwrap();

            // Compare the normalized JSON values
            assert_eq!(
                original,
                reserialized,
                "File {} failed round-trip serialization test",
                path.display()
            );
        }
    }
}
