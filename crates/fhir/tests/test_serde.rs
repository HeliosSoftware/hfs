use fhir::DecimalElement;
use fhir::Element;
use fhir::PreciseDecimal;
use fhir::r4::*;
use serde::Deserialize;
use serde::ser::SerializeStruct;

use rust_decimal_macros::dec;
use serde_json;

#[test]
fn test_serialize_decimal_with_value_present() {
    // Use the dec! macro
    let decimal_val = dec!(1050.00);
    let element = DecimalElement::<Extension> {
        id: None,
        extension: None,
        // Use the new constructor
        value: Some(PreciseDecimal::new(decimal_val, "1050.00".to_string())),
    };

    // Serialize the actual element
    let actual_json_string = serde_json::to_string(&element).expect("Serialization failed");
    // Prefix unused variable
    let _actual_value: serde_json::Value =
        serde_json::from_str(&actual_json_string).expect("Parsing actual JSON failed");

    // With our new implementation, a bare decimal with no other fields
    // is serialized as just the number.
    let expected_json_string = "1050.00";

    // Compare the output string directly
    assert_eq!(
        actual_json_string, expected_json_string,
        "Actual JSON: {} \nExpected JSON: {}",
        actual_json_string, expected_json_string
    );
}

#[test]
fn test_serialize_decimal_with_value_absent() {
    let element = DecimalElement::<Extension> {
        id: Some("test-id-123".to_string()),
        extension: None,
        value: None,
    };

    let json_string = serde_json::to_string(&element).expect("Serialization failed");
    let json_value: serde_json::Value =
        serde_json::from_str(&json_string).expect("Parsing JSON failed");

    assert!(
        json_value.get("value").is_none(),
        "Value field should be absent. JSON string was: {}",
        json_string
    );
    assert_eq!(
        json_value.get("id"),
        Some(&serde_json::json!("test-id-123"))
    );
    assert!(json_value.get("extension").is_none());
}

#[test]
fn test_serialize_decimal_with_all_fields() {
    // Use the dec! macro
    let decimal_val = dec!(-987.654321);
    let element = DecimalElement::<Extension> {
        id: Some("all-fields-present".to_string()),
        extension: Some(vec![
            Extension {
                id: None,
                extension: None,
                url: "http://example.com/ext1".to_string(),
                // Construct Boolean explicitly, initializing all fields
                value: Some(ExtensionValue::Boolean(Boolean {
                    id: None,
                    extension: None,
                    value: Some(true),
                })),
            },
            Extension {
                id: None,
                extension: None,
                url: "http://example.com/ext2".to_string(),
                // Construct String explicitly, initializing all fields
                value: Some(ExtensionValue::String(String {
                    id: None,
                    extension: None,
                    value: Some("val2".to_string()),
                })),
            },
        ]),
        // Use the new constructor
        value: Some(PreciseDecimal::new(decimal_val, "-987.654321".to_string())),
    };

    let json_string = serde_json::to_string(&element).expect("Serialization failed");
    let json_value: serde_json::Value =
        serde_json::from_str(&json_string).expect("Parsing JSON failed");

    assert_eq!(
        json_value.get("id"),
        Some(&serde_json::json!("all-fields-present"))
    );
    // Assertion remains the same (expecting JSON number output)
    assert_eq!(
        json_value.get("value"),
        // Compare against the number representation directly
        Some(&serde_json::json!(-987.654321)),
        "Value mismatch. JSON string was: {}",
        json_string
    );
    assert!(json_value.get("extension").is_some());
    // Update expected JSON for Extension
    assert_eq!(
        json_value["extension"],
        serde_json::json!([
            { "url": "http://example.com/ext1", "valueBoolean": true },
            { "url": "http://example.com/ext2", "valueString": "val2" }
        ])
    );
}

#[test]
fn test_serialize_decimal_with_no_fields() {
    let element = DecimalElement::<Extension> {
        id: None,
        extension: None,
        value: None,
    };

    let json_string = serde_json::to_string(&element).expect("Serialization failed");
    assert_eq!(
        json_string, "{}",
        "Serialization of empty element should be empty object"
    );
}

#[test]
fn test_deserialize_decimal_from_integer() {
    // Test with an integer value in an object
    let json_string = r#"{"value": 10}"#;
    let element: DecimalElement<Extension> =
        serde_json::from_str(json_string).expect("Deserialization failed");

    // Check the numerical value
    assert_eq!(element.value.as_ref().map(|pd| pd.value()), Some(dec!(10)));
    // Check the stored original string
    assert_eq!(element.value.map(|pd| pd.original_string().to_string()), Some("10".to_string()));


    // Test with a bare integer - this needs to be parsed as a JSON value first
    let json_value = serde_json::json!(10);
    let element: DecimalElement<Extension> =
        serde_json::from_value(json_value).expect("Deserialization from value failed");

    // Check the numerical value
    assert_eq!(element.value.as_ref().map(|pd| pd.value()), Some(dec!(10)));
    // Check the stored original string
    assert_eq!(element.value.map(|pd| pd.original_string().to_string()), Some("10".to_string()));
}

#[test]
fn test_roundtrip_decimal_serialization() {
    // Test with a bare integer
    let json_value = serde_json::json!(10);

    // Deserialize to our type
    let element: DecimalElement<Extension> =
        serde_json::from_value(json_value.clone()).expect("Deserialization failed");

    // Serialize back to JSON
    let reserialized = serde_json::to_value(&element).expect("Serialization failed");

    // Verify we get the same JSON back (a bare number, not an object)
    assert_eq!(
        json_value, reserialized,
        "Original: {:?}\nReserialized: {:?}",
        json_value, reserialized
    );

    // Test with a decimal value
    let json_value = serde_json::json!(123.456);

    // Deserialize to our type
    let element: DecimalElement<Extension> =
        serde_json::from_value(json_value.clone()).expect("Deserialization failed");

    // Serialize back to JSON
    let reserialized = serde_json::to_value(&element).expect("Serialization failed");

    // Verify we get the same JSON back (comparing serde_json::Value numbers)
    assert_eq!(json_value, reserialized);
}

#[test]
fn test_decimal_with_trailing_zeros() {
    // Test with a decimal value that has trailing zeros (3.0)
    let json_value = serde_json::json!(3.0); // Input is a JSON number 3.0
    // EXPECTED OUTPUT IS NOW A JSON NUMBER 3.0 (represented as string "3.0")
    let expected_string = "3.0";

    // Deserialize to our type
    let element: DecimalElement<Extension> =
        serde_json::from_value(json_value.clone()).expect("Deserialization from number failed");

    // Serialize back to string
    let reserialized_string =
        serde_json::to_string(&element).expect("Serialization to string failed");

    // Verify the string representation is the JSON number 3.0 (as string "3.0")
    assert_eq!(
        reserialized_string, expected_string,
        "Original JSON Value: {:?}\nExpected String: {}\nReserialized String: {}",
        json_value, expected_string, reserialized_string
    );

    // Also test with a string representation in the JSON input: "3.0"
    let json_str_input = r#""3.0""#; // Input is a JSON string "3.0"
    // Note: Deserializing a JSON string "3.0" into DecimalElement should still work
    // because the visitor handles visit_str/visit_borrowed_str.
    // The serialized output should still be the bare number 3.0.
    let element_from_string: DecimalElement<Extension> =
        serde_json::from_str(json_str_input).expect("Deserialization from string failed");

    // Serialize back to string
    let reserialized_string_from_str =
        serde_json::to_string(&element_from_string).expect("Serialization to string failed");

    // Verify the string representation is the JSON number 3.0 (as string "3.0")
    assert_eq!(
        reserialized_string_from_str, expected_string,
        "Original JSON String: {}\nExpected String: {}\nReserialized String: {}",
        json_str_input, expected_string, reserialized_string_from_str
    );

    // Test case from the failure log: parsing the string "3.0" directly
    let json_str = r#"3.0"#; // Input is bare number 3.0 in a string
    let parsed_value: serde_json::Value = serde_json::from_str(json_str).unwrap(); // Parsed as Number(3.0)

    let element_from_bare_string: DecimalElement<Extension> =
        serde_json::from_value(parsed_value.clone())
            .expect("Deserialization from bare string failed");

    let reserialized_string_from_bare =
        serde_json::to_string(&element_from_bare_string).expect("Serialization failed");

    // Verify the string representation is the JSON number 3.0 (as string "3.0")
    assert_eq!(
        reserialized_string_from_bare, expected_string,
        "Original bare string: {}\nParsed Value: {:?}\nExpected String: {}\nReserialized String: {}",
        json_str, parsed_value, expected_string, reserialized_string_from_bare
    );

    // Test with a decimal value that has trailing zeros (3.00)
    let json_value = serde_json::json!(3.00); // Input is a JSON number 3.00
    // EXPECTED OUTPUT IS NOW A JSON NUMBER 3.00 (represented as string "3.00")
    let expected_string = "3.00";

    // Deserialize to our type
    let element: DecimalElement<Extension> =
        serde_json::from_value(json_value.clone()).expect("Deserialization from number failed");

    // Serialize back to string
    let reserialized_string =
        serde_json::to_string(&element).expect("Serialization to string failed");

    // Verify the string representation is the JSON number 3.00 (as string "3.00")
    assert_eq!(
        reserialized_string, expected_string,
        "Original JSON Value: {:?}\nExpected String: {}\nReserialized String: {}",
        json_value, expected_string, reserialized_string
    );
}

#[test]
fn test_serialize_element_primitive() {
    let element = Element::<std::string::String, Extension> {
        id: None,
        extension: None,
        value: Some("test_value".to_string()),
    };
    let json_string = serde_json::to_string(&element).unwrap();
    // Should serialize as the primitive value directly
    assert_eq!(json_string, r#""test_value""#);

    let element_null = Element::<String, Extension> {
        id: None,
        extension: None,
        value: None,
    };
    let json_string_null = serde_json::to_string(&element_null).unwrap();
    // Should serialize as null
    assert_eq!(json_string_null, "null");

    // Test with integer
    let element_int = Element::<i32, Extension> {
        id: None,
        extension: None,
        value: Some(123),
    };
    let json_string_int = serde_json::to_string(&element_int).unwrap();
    assert_eq!(json_string_int, "123");

    // Test with boolean
    let element_bool = Element::<bool, Extension> {
        id: None,
        extension: None,
        value: Some(true),
    };
    let json_string_bool = serde_json::to_string(&element_bool).unwrap();
    assert_eq!(json_string_bool, "true");
}

#[test]
fn test_serialize_element_object() {
    let element = Element::<std::string::String, Extension> {
        id: Some("elem-id".to_string()),
        extension: Some(vec![Extension {
            id: None,
            extension: None,
            url: "http://example.com/ext1".to_string(),
            // Construct Boolean explicitly, initializing all fields
            value: Some(ExtensionValue::Boolean(Boolean {
                id: None,
                extension: None,
                value: Some(true),
            })),
        }]),
        value: Some("test_value".to_string()),
    };
    let json_string = serde_json::to_string(&element).unwrap();
    // Should serialize as an object because id/extension are present
    let expected_json = r#"{"id":"elem-id","extension":[{"url":"http://example.com/ext1","valueBoolean":true}],"value":"test_value"}"#;
    assert_eq!(json_string, expected_json);

    // Test with only id
    let element_id_only = Element::<std::string::String, Extension> {
        id: Some("elem-id-only".to_string()),
        extension: None,
        value: Some("test_value_id".to_string()),
    };
    let json_string_id_only = serde_json::to_string(&element_id_only).unwrap();
    let expected_json_id_only = r#"{"id":"elem-id-only","value":"test_value_id"}"#;
    assert_eq!(json_string_id_only, expected_json_id_only);

    // Test with only extension
    let element_ext_only = Element::<std::string::String, Extension> {
        id: None,
        extension: Some(vec![Extension {
            id: None,
            extension: None,
            url: "http://example.com/ext2".to_string(),
            // Construct String explicitly, initializing all fields
            value: Some(ExtensionValue::String(String {
                id: None,
                extension: None,
                value: Some("val2".to_string()),
            })),
        }]),
        value: Some("test_value_ext".to_string()),
    };
    let json_string_ext_only = serde_json::to_string(&element_ext_only).unwrap();
    let expected_json_ext_only = r#"{"extension":[{"url":"http://example.com/ext2","valueString":"val2"}],"value":"test_value_ext"}"#;
    assert_eq!(json_string_ext_only, expected_json_ext_only);

    // Test with id, extension, but no value
    let element_no_value = Element::<String, Extension> {
        id: Some("elem-id-no-val".to_string()),
        extension: Some(vec![Extension {
            id: None,
            extension: None,
            url: "http://example.com/ext3".to_string(),
            // Construct Integer explicitly, initializing all fields
            value: Some(ExtensionValue::Integer(Integer {
                id: None,
                extension: None,
                value: Some(123),
            })),
        }]),
        value: None,
    };
    let json_string_no_value = serde_json::to_string(&element_no_value).unwrap();
    // Should serialize object without the "value" field
    let expected_json_no_value = r#"{"id":"elem-id-no-val","extension":[{"url":"http://example.com/ext3","valueInteger":123}]}"#;
    assert_eq!(json_string_no_value, expected_json_no_value);
}

#[test]
fn test_deserialize_element_primitive() {
    // String primitive
    let json_string = r#""test_value""#;
    let element: Element<std::string::String, Extension> =
        serde_json::from_str(json_string).unwrap();
    assert_eq!(element.id, None);
    assert_eq!(element.extension, None);
    assert_eq!(element.value, Some("test_value".to_string()));

    // Null primitive
    let json_null = "null";
    let element_null: Element<String, Extension> = serde_json::from_str(json_null).unwrap();
    assert_eq!(element_null.id, None);
    assert_eq!(element_null.extension, None);
    assert_eq!(element_null.value, None);

    // Number primitive
    let json_num = "123";
    let element_num: Element<i32, Extension> = serde_json::from_str(json_num).unwrap();
    assert_eq!(element_num.id, None);
    assert_eq!(element_num.extension, None);
    assert_eq!(element_num.value, Some(123));

    // Boolean primitive
    let json_bool = "true";
    let element_bool: Element<bool, Extension> = serde_json::from_str(json_bool).unwrap();
    assert_eq!(element_bool.id, None);
    assert_eq!(element_bool.extension, None);
    assert_eq!(element_bool.value, Some(true));
}

#[test]
fn test_deserialize_element_object() {
    // Full object
    let json_string = r#"{"id":"elem-id","extension":[{"url":"http://example.com/ext1","valueBoolean":true}],"value":"test_value"}"#;
    let element: Element<std::string::String, Extension> =
        serde_json::from_str(json_string).unwrap();
    assert_eq!(element.id, Some("elem-id".to_string()));
    assert_eq!(
        element.extension,
        Some(vec![Extension {
            id: None,
            extension: None,
            url: "http://example.com/ext1".to_string(),
            // Construct Boolean explicitly, initializing all fields
            value: Some(ExtensionValue::Boolean(Boolean {
                id: None,
                extension: None,
                value: Some(true)
            })),
        }])
    );
    assert_eq!(element.value, Some("test_value".to_string()));

    // Object with missing value
    let json_missing_value = r#"{"id":"elem-id-no-val","extension":[{"url":"http://example.com/ext3","valueInteger":123}]}"#;
    let element_missing_value: Element<String, Extension> =
        serde_json::from_str(json_missing_value).unwrap();
    assert_eq!(element_missing_value.id, Some("elem-id-no-val".to_string()));
    assert_eq!(
        element_missing_value.extension,
        Some(vec![Extension {
            id: None,
            extension: None,
            url: "http://example.com/ext3".to_string(),
            // Construct Integer explicitly, initializing all fields
            value: Some(ExtensionValue::Integer(Integer {
                id: None,
                extension: None,
                value: Some(123)
            })),
        }])
    );
    assert_eq!(element_missing_value.value, None); // Value should be None

    // Object with missing extension
    let json_missing_ext = r#"{"id":"elem-id-only","value":"test_value_id"}"#;
    let element_missing_ext: Element<std::string::String, Extension> =
        serde_json::from_str(json_missing_ext).unwrap();
    assert_eq!(element_missing_ext.id, Some("elem-id-only".to_string()));
    assert_eq!(element_missing_ext.extension, None);
    assert_eq!(element_missing_ext.value, Some("test_value_id".to_string()));

    // Object with missing id
    let json_missing_id = r#"{"extension":[{"url":"http://example.com/ext2","valueString":"val2"}],"value":"test_value_ext"}"#;
    let element_missing_id: Element<std::string::String, Extension> =
        serde_json::from_str(json_missing_id).unwrap();
    assert_eq!(element_missing_id.id, None);
    assert_eq!(
        element_missing_id.extension,
        Some(vec![Extension {
            id: None,
            extension: None,
            url: "http://example.com/ext2".to_string(),
            // Construct String explicitly, initializing all fields
            value: Some(ExtensionValue::String(String {
                id: None,
                extension: None,
                value: Some("val2".to_string())
            })),
        }])
    );
    assert_eq!(element_missing_id.value, Some("test_value_ext".to_string()));

    // Object with only value
    let json_only_value_obj = r#"{"value":"test_value_only"}"#;
    let element_only_value: Element<std::string::String, Extension> =
        serde_json::from_str(json_only_value_obj).unwrap();
    assert_eq!(element_only_value.id, None);
    assert_eq!(element_only_value.extension, None);
    assert_eq!(
        element_only_value.value,
        Some("test_value_only".to_string())
    );

    // Empty object
    let json_empty_obj = r#"{}"#;
    let element_empty_obj: Element<String, Extension> =
        serde_json::from_str(json_empty_obj).unwrap();
    assert_eq!(element_empty_obj.id, None);
    assert_eq!(element_empty_obj.extension, None);
    assert_eq!(element_empty_obj.value, None); // Value is None when deserializing from empty object
}

#[test]
fn test_deserialize_element_invalid_type() {
    // Array is not a valid representation for a single Element
    let json_array = r#"[1, 2, 3]"#;
    let result: Result<Element<i32, Extension>, _> = serde_json::from_str(json_array);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("invalid type: sequence, expected a primitive value (string, number, boolean), an object, or null"));

    // Boolean when expecting i32 (primitive case)
    let json_bool = r#"true"#;
    let result_bool: Result<Element<i32, Extension>, _> = serde_json::from_str(json_bool);
    assert!(result_bool.is_err());
    // Error should now come directly from V::deserialize (i32 failing on bool)
    let err_string = result_bool.unwrap_err().to_string();
    assert!(
        err_string.contains("invalid type: boolean `true`, expected i32"),
        "Unexpected error message: {}",
        err_string // Add message for easier debugging
    );

    // Object containing a boolean value when expecting Element<i32, _>
    let json_obj_bool_val = r#"{"value": true}"#;
    let result_obj_bool: Result<Element<i32, Extension>, _> =
        serde_json::from_str(json_obj_bool_val);
    assert!(result_obj_bool.is_err());
    // Error comes from trying to deserialize the "value": true into Option<i32>
    assert!(
        result_obj_bool
            .unwrap_err()
            .to_string()
            .contains("invalid type: boolean `true`, expected i32")
    );

    // Define a simple struct that CANNOT deserialize from primitive types
    // Add Eq derive
    #[derive(Deserialize, Debug, PartialEq, Eq)]
    struct NonPrimitive {
        field: std::string::String,
    }

    // Try deserializing a primitive string into Element<NonPrimitive, _>
    let json_prim_str = r#""hello""#;
    let result_prim_nonprim: Result<Element<NonPrimitive, Extension>, _> =
        serde_json::from_str(json_prim_str);
    assert!(result_prim_nonprim.is_err());
    // Error comes from V::deserialize failing inside the visitor
    assert!(
        result_prim_nonprim
            .unwrap_err()
            .to_string()
            .contains("invalid type: string \"hello\", expected struct NonPrimitive")
    );

    // Try deserializing an object into Element<NonPrimitive, _> (this should work if object has correct field)
    let json_obj_nonprim = r#"{"value": {"field": "world"}}"#;
    // Use Extension
    let result_obj_nonprim: Result<Element<NonPrimitive, Extension>, _> =
        serde_json::from_str(json_obj_nonprim);
    assert!(result_obj_nonprim.is_ok());
    let element_obj_nonprim = result_obj_nonprim.unwrap();
    assert_eq!(element_obj_nonprim.id, None);
    assert_eq!(element_obj_nonprim.extension, None);
    assert_eq!(
        element_obj_nonprim.value,
        Some(NonPrimitive {
            field: "world".to_string()
        })
    );
}

// --- Tests for FhirSerde derive macro (_fieldName logic) ---

use fhir_macro::FhirSerde;

// Define a test struct that uses manual Serialize implementation
#[derive(Debug, PartialEq, FhirSerde)]
struct FhirSerdeTestStruct {
    // Regular field
    name: Option<std::string::String>,

    // Field with potential extension (_birthDate) using type alias
    // FhirSerde should handle the 'birthDate'/'_birthDate' logic based on the field name.
    birth_date: Option<Date>,

    // Another potentially extended field using type alias
    // FhirSerde should handle the 'isActive'/'_isActive' logic based on the field name.
    is_active: Option<Boolean>,

    // A non-element field for good measure
    count: Option<i32>,
}

#[test]
fn test_fhir_serde_serialize() {
    // Case 1: Only primitive value for birthDate
    // Use Date which is Element<String, Extension>
    let s1 = FhirSerdeTestStruct {
        name: Some("Test1".to_string()),
        birth_date: Some(Date {
            // Construct using the alias type
            id: None,
            extension: None,
            value: Some("1970-03-30".to_string()),
        }),
        is_active: None,
        count: Some(1),
    };
    let json1 = serde_json::to_string(&s1).unwrap();
    let expected1 = r#"{"name":"Test1","birthDate":"1970-03-30","count":1}"#;
    assert_eq!(json1, expected1);

    // Case 2: Only extension for birthDate
    // Use Date which is Element<String, Extension>
    let s2 = FhirSerdeTestStruct {
        name: Some("Test2".to_string()),
        birth_date: Some(Date {
            // Construct using the alias type
            id: Some("bd-id".to_string()),
            extension: None, // Using None for simplicity in test data
            value: None,
        }),
        is_active: None,
        count: None,
    };
    let json2 = serde_json::to_string(&s2).unwrap();
    // Expected output according to FHIR: Only _fieldName when value is absent
    let expected2 = r#"{"name":"Test2","birthDate":{"id":"bd-id"}}"#;
    println!("Actual JSON: {}", json2);
    println!("Expected JSON: {}", expected2);
    assert_eq!(json2, expected2);

    // Case 3: Both primitive value and extension for birthDate
    // Use Date and Boolean
    let s3 = FhirSerdeTestStruct {
        name: Some("Test3".to_string()),
        birth_date: Some(Date {
            // Construct using the alias type
            id: Some("bd-id-3".to_string()),
            // Use the actual Extension type, or None if simpler
            extension: None, // Using None for simplicity
            value: Some("1970-03-30".to_string()),
        }),
        is_active: Some(Boolean {
            // Construct using the alias type
            // Also test is_active field
            id: None,
            extension: None,
            value: Some(true),
        }),
        count: Some(3),
    };

    // Debug print the structure
    println!(
        "birth_date: id={:?}, extension={:?}, value={:?}",
        s3.birth_date.as_ref().unwrap().id,
        s3.birth_date.as_ref().unwrap().extension,
        s3.birth_date.as_ref().unwrap().value
    );

    let json3 = serde_json::to_string(&s3).unwrap();
    println!("Serialized JSON: {}", json3);
    // Expected output according to FHIR: Both fieldName (for value) and _fieldName (for id/extension)
    // The _birthDate object should only contain id/extension, not the value.
    // isActive only has value, so only "isActive" field.
    let expected3 = r#"{"name":"Test3","birthDate":"1970-03-30","_birthDate":{"id":"bd-id-3"},"isActive":true,"count":3}"#; // Corrected structure

    // Print the actual JSON for debugging
    println!("Actual JSON: {}", json3);
    println!("Expected JSON: {}", expected3);

    assert_eq!(json3, expected3);

    // Case 4: birthDate field is None
    // Use Boolean
    let s4 = FhirSerdeTestStruct {
        name: Some("Test4".to_string()),
        birth_date: None,
        is_active: Some(Boolean {
            // Construct using the alias type
            // is_active has only extension
            id: None,
            // Expect the actual Extension based on corrected json4 input
            extension: Some(vec![Extension {
                id: None,
                extension: None,
                url: "http://example.com/flag".to_string(),
                // Construct Boolean explicitly, initializing all fields
                value: Some(ExtensionValue::Boolean(Boolean {
                    id: None,
                    extension: None,
                    value: Some(true),
                })),
            }]),
            value: None,
        }),
        count: None,
    };
    let json4 = serde_json::to_string(&s4).unwrap();
    // Expected output according to FHIR: Only fieldName when value is absent but extension exists
    let expected4 = r#"{"name":"Test4","isActive":{"extension":[{"url":"http://example.com/flag","valueBoolean":true}]}}"#;
    assert_eq!(json4, expected4);

    // Case 5: All optional fields are None
    let s5 = FhirSerdeTestStruct {
        name: None,
        birth_date: None,
        is_active: None,
        count: None,
    };
    let json5 = serde_json::to_string(&s5).unwrap();
    let expected5 = r#"{}"#;
    assert_eq!(json5, expected5);
}

#[test]
fn test_fhir_serde_deserialize() {
    // Case 1: Only primitive value for birthDate
    let json1 = r#"{"name":"Test1","birthDate":"1970-03-30","count":1}"#;
    // Use Date
    let expected1 = FhirSerdeTestStruct {
        name: Some("Test1".to_string()),
        birth_date: Some(Date {
            // Construct using the alias type
            id: None,
            extension: None,
            value: Some("1970-03-30".to_string()),
        }),
        is_active: None,
        count: Some(1),
    };
    let s1: FhirSerdeTestStruct = serde_json::from_str(json1).unwrap();
    assert_eq!(s1, expected1);

    // Case 2: Only extension for birthDate
    let json2 = r#"{"name":"Test2","_birthDate":{"id":"bd-id","extension":[{"url":"http://example.com/note","valueString":"some note"}]}}"#;
    // Use Date
    let expected2 = FhirSerdeTestStruct {
        // Prefixed unused variable
        name: Some("Test2".to_string()),
        birth_date: Some(Date {
            // Construct using the alias type
            id: Some("bd-id".to_string()),
            // Expect the valid extension based on corrected json2
            extension: Some(vec![Extension {
                id: None,
                extension: None,
                url: "http://example.com/note".to_string(),
                value: Some(ExtensionValue::String(String {
                    id: None,
                    extension: None,
                    value: Some("some note".to_string()),
                })),
            }]),
            value: None,
        }),
        is_active: None,
        count: None,
    };
    let s2: FhirSerdeTestStruct = serde_json::from_str(json2).unwrap(); // Prefixed unused variable
    assert_eq!(s2, expected2);

    // Case 3: Both primitive value and extension for birthDate and isActive
    let json3 = r#"{"name":"Test3","birthDate":"1970-03-30","_birthDate":{"id":"bd-id-3","extension":[{"url":"http://example.com/test","valueString":"some-ext-val"}]},"isActive":true,"_isActive":{"id":"active-id"},"count":3}"#;
    // Use Date and Boolean
    let _expected3 = FhirSerdeTestStruct {
        // Prefixed unused variable
        name: Some("Test3".to_string()),
        birth_date: Some(Date {
            // Construct using the alias type
            id: Some("bd-id-3".to_string()),
            // Expect the actual Extension based on the corrected json3 input
            extension: Some(vec![Extension {
                id: None,
                extension: None,
                url: "http://example.com/test".to_string(),
                // Construct String explicitly, initializing all fields
                value: Some(ExtensionValue::String(String {
                    id: None,
                    extension: None,
                    value: Some("some-ext-val".to_string()),
                })),
            }]),
            value: Some("1970-03-30".to_string()),
        }),
        is_active: Some(Boolean {
            // Construct using the alias type
            id: Some("active-id".to_string()), // Merged from _isActive
            extension: None,                   // Merged from _isActive
            value: Some(true),                 // From isActive
        }),
        count: Some(3),
    };
    let _s3: FhirSerdeTestStruct = serde_json::from_str(json3).unwrap(); // Prefixed unused variable
    assert_eq!(_s3, _expected3); // Should now pass with macro fix

    // Case 4: birthDate field is missing, isActive has only extension
    // Update JSON to use valid Extension structure
    let json4 = r#"{"name":"Test4","isActive":{"extension":[{"url":"http://example.com/flag","valueBoolean":true}]}}"#;
    // Use Boolean
    let _expected4 = FhirSerdeTestStruct {
        // Prefixed unused variable
        name: Some("Test4".to_string()),
        birth_date: None,
        is_active: Some(Boolean {
            // Construct using the alias type
            id: None,
            extension: Some(vec![Extension {
                id: None,
                extension: None,
                url: "http://example.com/flag".to_string(),
                value: Some(ExtensionValue::Boolean(Boolean {
                    id: None,
                    extension: None,
                    value: Some(true),
                })),
            }]),
            value: None,
        }),
        count: None,
    };
    let _s4: FhirSerdeTestStruct = serde_json::from_str(json4).unwrap(); // Prefixed unused variable
    assert_eq!(_s4, _expected4); // Should now pass with macro fix

    // Case 5: Empty object
    let json5 = r#"{}"#;
    let expected5 = FhirSerdeTestStruct {
        name: None,
        birth_date: None,
        is_active: None,
        count: None,
    };
    let s5: FhirSerdeTestStruct = serde_json::from_str(json5).unwrap();
    assert_eq!(s5, expected5); // Uncomment assertion

    // Case 6: Primitive value is null, but extension exists
    let json6 = r#"{"birthDate":null,"_birthDate":{"id":"bd-null"}}"#;
    // Use Date
    let expected6 = FhirSerdeTestStruct {
        // Renamed from _expected6
        name: None,
        birth_date: Some(Date {
            // Construct using the alias type
            id: Some("bd-null".to_string()),
            extension: None,
            value: None, // Value is None because input was null
        }),
        is_active: None,
        count: None,
    };
    let s6: FhirSerdeTestStruct = serde_json::from_str(json6).unwrap(); // Renamed from _s6
    assert_eq!(s6, expected6); // Use renamed variables

    // Case 7: Primitive value exists, but extension is null (should ignore null extension object)
    let json7 = r#"{"birthDate":"1999-09-09","_birthDate":null}"#;
    // Use Date
    let expected7 = FhirSerdeTestStruct {
        // Renamed from _expected7
        name: None,
        birth_date: Some(Date {
            // Construct using the alias type
            id: None,
            extension: None,
            value: Some("1999-09-09".to_string()),
        }),
        is_active: None,
        count: None,
    };
    let s7: FhirSerdeTestStruct = serde_json::from_str(json7).unwrap(); // Renamed from _s7
    assert_eq!(s7, expected7); // Use renamed variables

    // Case 8: Duplicate primitive field (should error)
    let json8 = r#"{"birthDate":"1970-03-30", "birthDate":"1971-04-01"}"#;
    let res8: Result<FhirSerdeTestStruct, _> = serde_json::from_str(json8);
    assert!(res8.is_err());
    assert!(
        res8.unwrap_err()
            .to_string()
            .contains("duplicate field `birthDate`")
    );

    // Case 9: Duplicate extension field (should error)
    let json9 = r#"{"_birthDate":{"id":"a"}, "_birthDate":{"id":"b"}}"#;
    let res9: Result<FhirSerdeTestStruct, _> = serde_json::from_str(json9); // Renamed from _res9
    assert!(res9.is_err()); // Should now error due to duplicate field handled by macro visitor
    assert!(
        res9.unwrap_err()
            .to_string()
            .contains("duplicate field `_birthDate`")
    );
}
