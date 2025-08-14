#[cfg(feature = "R6")]
#[test]
fn test_simple_null_handling() {
    // Test a simple case of deserializing JSON with null
    let json_with_null = r#"{"conclusionCode": [null]}"#;

    let result: Result<serde_json::Value, _> = serde_json::from_str(json_with_null);
    assert!(result.is_ok(), "Should be able to parse JSON with null");

    let value = result.unwrap();
    println!("Parsed JSON: {:?}", value);

    // Check that we can access the array
    if let Some(array) = value.get("conclusionCode").and_then(|v| v.as_array()) {
        println!("Array has {} elements", array.len());
        for (i, elem) in array.iter().enumerate() {
            if elem.is_null() {
                println!("  [{}]: null", i);
            } else {
                println!("  [{}]: {:?}", i, elem);
            }
        }
    }
}
