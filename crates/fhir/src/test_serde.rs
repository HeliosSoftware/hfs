use helios_fhir::DecimalElement;
use helios_fhir::Element;
use helios_fhir::PreciseDecimal;
use helios_fhir::PrecisionDate;
use helios_fhir::r4::*;
use rust_decimal_macros::dec;
use serde::Deserialize;

#[test]
fn test_serialize_decimal_with_value_present() {
    // Use the dec! macro
    let decimal_val = dec!(1050.00);
    let element = DecimalElement::<Extension> {
        id: None,
        extension: None,
        // Use from_parts constructor
        value: Some(PreciseDecimal::from_parts(
            Some(decimal_val),
            "1050.00".to_string(),
        )),
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

// --- Test for deserializing array with null primitive and corresponding extension ---

// Define a struct to hold the Timing element for testing
#[derive(Debug, PartialEq, FhirSerde)]
struct TimingTestStruct {
    #[fhir_serde(rename = "timingTiming")]
    timing_timing: Option<Timing>, // Use Option<Timing> to match the JSON structure
}

#[test]
fn test_deserialize_timing_with_null_primitive_and_extension() {
    let json_input = r#"
    {
      "timingTiming": {
        "event": [
          null
        ],
        "_event": [
          {
            "extension": [
              {
                "url": "http://hl7.org/fhir/StructureDefinition/cqf-expression",
                "valueExpression": {
                  "language": "text/cql",
                  "expression": "Now()"
                }
              }
            ]
          }
        ]
      }
    }
    "#;

    // Expected Rust structure after deserialization
    let expected_struct = TimingTestStruct {
        timing_timing: Some(Timing {
            id: None,
            extension: None,
            modifier_extension: None,
            event: Some(vec![
                // The first element corresponds to the null primitive and its extension
                DateTime {
                    id: None,
                    // The extension comes from the corresponding _event element
                    extension: Some(vec![Extension {
                        id: None,
                        extension: None,
                        url: "http://hl7.org/fhir/StructureDefinition/cqf-expression"
                            .to_string()
                            .into(),
                        value: Some(ExtensionValue::Expression(Expression {
                            id: None,
                            extension: None,
                            description: None,
                            name: None,
                            language: Code {
                                id: None,
                                extension: None,
                                value: Some("text/cql".to_string()),
                            },
                            expression: Some(String {
                                id: None,
                                extension: None,
                                value: Some("Now()".to_string()),
                            }),
                            reference: None,
                        })),
                    }]),
                    // The value is None because the primitive in the 'event' array was null
                    value: None,
                },
            ]),
            repeat: None,
            code: None,
        }),
    };

    // Deserialize the JSON input
    let deserialized_struct: TimingTestStruct =
        serde_json::from_str(json_input).expect("Deserialization failed");

    // Assert that the deserialized struct matches the expected structure
    assert_eq!(
        deserialized_struct, expected_struct,
        "Deserialized struct does not match expected structure.\nExpected: {:#?}\nActual: {:#?}",
        expected_struct, deserialized_struct
    );
}

#[test]
fn test_decimal_out_of_range() {
    // Test values from observation-decimal.json that are outside rust_decimal::Decimal range
    let json_inputs = [
        "1E-22",                      // Small, but in range
        "1.000000000000000000E-245",  // Too small
        "-1.000000000000000000E+245", // Too large (negative)
    ];

    for json_input in json_inputs {
        // Deserialize from string
        let element: DecimalElement<Extension> = serde_json::from_str(json_input)
            .unwrap_or_else(|e| panic!("Deserialization from '{}' failed: {}", json_input, e));

        // Check that the original string was preserved exactly
        assert_eq!(
            element.value.as_ref().map(|pd| pd.original_string()),
            Some(json_input), // Expect the exact original input string
            "Stored original string mismatch for input: {}",
            json_input
        );

        // Check that the parsed Decimal value is None for out-of-range inputs
        if json_input == "1E-22" {
            // 1E-22 should parse correctly
            assert!(
                element.value.as_ref().and_then(|pd| pd.value()).is_some(),
                "Parsed value should be Some for input: {}",
                json_input
            );
            assert_eq!(
                element.value.as_ref().and_then(|pd| pd.value()),
                Some(dec!(1E-22)),
                "Parsed value mismatch for input: {}",
                json_input
            );
        } else {
            // E-245 and E+245 should result in None
            assert!(
                element.value.as_ref().and_then(|pd| pd.value()).is_none(),
                "Parsed value should be None for out-of-range input: {}",
                json_input
            );
        }

        // Serialize back to string
        let reserialized_string = serde_json::to_string(&element).unwrap_or_else(|e| {
            panic!(
                "Serialization back to string for input '{}' failed: {}",
                json_input, e
            )
        });

        // Verify the original string representation is output
        assert_eq!(
            reserialized_string, json_input,
            "Roundtrip string mismatch for input: {}",
            json_input
        );

        // Serialize back to JSON Value
        let reserialized_value = serde_json::to_value(&element).unwrap_or_else(|e| {
            panic!(
                "Serialization back to value for input '{}' failed: {}",
                json_input, e
            )
        });

        // Verify the reserialized value matches the original input number/string
        // We need to parse the original input string into a Value for comparison
        let original_value: serde_json::Value = serde_json::from_str(json_input).unwrap();
        assert_eq!(
            reserialized_value, original_value,
            "Roundtrip value mismatch for input: {}",
            json_input
        );
    }
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
                url: "http://example.com/ext1".to_string().into(), // Convert String to Url (Element<String, Extension>)
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
                url: "http://example.com/ext2".to_string().into(), // Convert String to Url
                // Construct String explicitly, initializing all fields
                value: Some(ExtensionValue::String(String {
                    id: None,
                    extension: None,
                    value: Some("val2".to_string()),
                })),
            },
        ]),
        // Use from_parts constructor
        value: Some(PreciseDecimal::from_parts(
            Some(decimal_val),
            "-987.654321".to_string(),
        )),
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
        json_string, "null",
        "Serialization of empty element should be null"
    );
}

#[test]
fn test_deserialize_decimal_from_integer() {
    // Test with an integer value in an object
    let json_string = r#"{"value": 10}"#;
    let element: DecimalElement<Extension> =
        serde_json::from_str(json_string).expect("Deserialization failed");

    // Check the numerical value (pd.value() now returns Option<Decimal>)
    assert_eq!(
        element.value.as_ref().and_then(|pd| pd.value()),
        Some(dec!(10))
    );
    // Check the stored original string
    assert_eq!(
        element
            .value
            .as_ref()
            .map(|pd| pd.original_string().to_string()),
        Some("10".to_string())
    );

    // Test with a bare integer string
    let json_string = r#"10"#;
    let element: DecimalElement<Extension> =
        serde_json::from_str(json_string).expect("Deserialization from bare integer string failed");

    // Check the numerical value (pd.value() now returns Option<Decimal>)
    assert_eq!(
        element.value.as_ref().and_then(|pd| pd.value()),
        Some(dec!(10))
    );
    // Check the stored original string
    assert_eq!(
        element
            .value
            .as_ref()
            .map(|pd| pd.original_string().to_string()),
        Some("10".to_string())
    );
}

#[test]
fn test_roundtrip_decimal_serialization() {
    // Test with a bare integer string
    let json_input = "10";
    let expected_value = serde_json::json!(10); // Expected output is a JSON number

    // Deserialize from string
    let element: DecimalElement<Extension> =
        serde_json::from_str(json_input).expect("Deserialization from integer string failed");

    // Serialize back to JSON Value
    let reserialized_value = serde_json::to_value(&element).expect("Serialization to value failed");

    // Verify we get the expected JSON number value back
    assert_eq!(
        expected_value, reserialized_value,
        "Original String: {}\nExpected Value: {:?}\nReserialized Value: {:?}",
        json_input, expected_value, reserialized_value
    );

    // Test with a decimal value string
    let json_input = "123.456";
    let expected_value = serde_json::json!(123.456); // Expected output is a JSON number

    // Deserialize from string
    let element: DecimalElement<Extension> =
        serde_json::from_str(json_input).expect("Deserialization from decimal string failed");

    // Serialize back to JSON Value
    let reserialized_value = serde_json::to_value(&element).expect("Serialization to value failed");

    // Verify we get the expected JSON number value back
    assert_eq!(
        expected_value, reserialized_value,
        "Original String: {}\nExpected Value: {:?}\nReserialized Value: {:?}",
        json_input, expected_value, reserialized_value
    );
}

#[test]
fn test_decimal_with_trailing_zeros() {
    // Test case 1: Input is the JSON number 3.0
    let json_input_num_3_0 = "3.0";
    let expected_string_3_0 = "3.0";

    // Deserialize from string
    let element_num_3_0: DecimalElement<Extension> =
        serde_json::from_str(json_input_num_3_0).expect("Deserialization from '3.0' failed");

    // Serialize back to string
    let reserialized_string_num_3_0 =
        serde_json::to_string(&element_num_3_0).expect("Serialization to string failed");

    // Verify the string representation is "3.0"
    assert_eq!(
        reserialized_string_num_3_0, expected_string_3_0,
        "Input JSON: {}\nExpected String: {}\nReserialized String: {}",
        json_input_num_3_0, expected_string_3_0, reserialized_string_num_3_0
    );

    // Test case 2: Input is the JSON string "3.0"
    let json_input_str_3_0 = r#""3.0""#; // Note the outer quotes for JSON string
    // The output should still be the JSON number 3.0
    let expected_string_3_0 = "3.0";

    // Deserialize from string
    let element_str_3_0: DecimalElement<Extension> =
        serde_json::from_str(json_input_str_3_0).expect("Deserialization from '\"3.0\"' failed");

    // Serialize back to string
    let reserialized_string_str_3_0 =
        serde_json::to_string(&element_str_3_0).expect("Serialization to string failed");

    // Verify the string representation is "3.0"
    assert_eq!(
        reserialized_string_str_3_0, expected_string_3_0,
        "Input JSON: {}\nExpected String: {}\nReserialized String: {}",
        json_input_str_3_0, expected_string_3_0, reserialized_string_str_3_0
    );

    // Test case 3: Input is the JSON number 3.00
    let json_input_num_3_00 = "3.00";
    let expected_string_3_00 = "3.00";

    // Deserialize from string
    let element_num_3_00: DecimalElement<Extension> =
        serde_json::from_str(json_input_num_3_00).expect("Deserialization from '3.00' failed");

    // Serialize back to string
    let reserialized_string_num_3_00 =
        serde_json::to_string(&element_num_3_00).expect("Serialization to string failed");

    // Verify the string representation is "3.00"
    assert_eq!(
        reserialized_string_num_3_00, expected_string_3_00,
        "Input JSON: {}\nExpected String: {}\nReserialized String: {}",
        json_input_num_3_00, expected_string_3_00, reserialized_string_num_3_00
    );

    // Test case 4: Input is the JSON string "3.00"
    let json_input_str_3_00 = r#""3.00""#; // Note the outer quotes for JSON string
    // The output should still be the JSON number 3.00
    let expected_string_3_00 = "3.00";

    // Deserialize from string
    let element_str_3_00: DecimalElement<Extension> =
        serde_json::from_str(json_input_str_3_00).expect("Deserialization from '\"3.00\"' failed");

    // Serialize back to string
    let reserialized_string_str_3_00 =
        serde_json::to_string(&element_str_3_00).expect("Serialization to string failed");

    // Verify the string representation is "3.00"
    assert_eq!(
        reserialized_string_str_3_00, expected_string_3_00,
        "Input JSON: {}\nExpected String: {}\nReserialized String: {}",
        json_input_str_3_00, expected_string_3_00, reserialized_string_str_3_00
    );

    // Test case 5: Input is the JSON number 3 (integer)
    let json_input_num_3 = "3";
    let expected_string_3 = "3";

    // Deserialize from string
    let element_num_3: DecimalElement<Extension> =
        serde_json::from_str(json_input_num_3).expect("Deserialization from '3' failed");

    // Serialize back to string
    let reserialized_string_num_3 =
        serde_json::to_string(&element_num_3).expect("Serialization to string failed");

    // Verify the string representation is "3"
    assert_eq!(
        reserialized_string_num_3, expected_string_3,
        "Input JSON: {}\nExpected String: {}\nReserialized String: {}",
        json_input_num_3, expected_string_3, reserialized_string_num_3
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
            url: "http://example.com/ext1".to_string().into(), // Convert String to Url
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
            url: "http://example.com/ext2".to_string().into(), // Convert String to Url
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
            url: "http://example.com/ext3".to_string().into(), // Convert String to Url
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
            url: "http://example.com/ext1".to_string().into(), // Convert String to Url
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
            url: "http://example.com/ext3".to_string().into(), // Convert String to Url
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
            url: "http://example.com/ext2".to_string().into(), // Convert String to Url
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

use helios_fhir_macro::FhirSerde;

// Define a test struct that uses manual Serialize implementation
#[derive(Debug, PartialEq, FhirSerde)]
struct FhirSerdeTestStruct {
    // Regular fields
    name1: String,
    name2: Option<String>,

    // Field with potential extension (_birthDate1) using type alias
    // FhirSerde should handle the 'birthDate1'/'_birthDate1' logic based on the field name.
    #[fhir_serde(rename = "birthDate1")]
    birth_date1: Date,
    #[fhir_serde(rename = "birthDate2")]
    birth_date2: Option<Date>,

    // Another potentially extended field using type alias
    // FhirSerde should handle the 'isActive1'/'_isActive1' logic based on the field name.
    #[fhir_serde(rename = "isActive1")]
    is_active1: Boolean,
    #[fhir_serde(rename = "isActive2")]
    is_active2: Option<Boolean>,

    // A field with potental extension (_decimal1) using type alias.
    // FhirSerde should handle the 'decimal1'/'_decimal1' logic based on the field name.
    decimal1: Decimal,
    decimal2: Option<Decimal>,

    // A field with potential extension (_moneyi1) that also has a Decimal value in it
    // FhirSerde should handle the 'money1'/'_money1' logic based on the field name.
    money1: Money,
    money2: Option<Money>,

    // A field that uses Vec - need to handle nulls in extensions correctly - https://hl7.org/fhir/json.html#primitive
    given: Option<Vec<String>>,
}

#[test]
fn test_helios_fhir_serde_serialize() {
    // Helper to create default extension
    let default_extension = || Extension {
        id: None,
        extension: None,
        url: "http://example.com/ext".to_string().into(), // Convert String to Url
        value: Some(ExtensionValue::String(String {
            id: None,
            extension: None,
            value: Some("ext-val".to_string()),
        })),
    };

    let decimal = Decimal::new(dec!(123.45));
    // Case 1: Only primitive value for birthDate1
    let s1 = FhirSerdeTestStruct {
        name1: "Test1".to_string().into(), // Required field
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let json1 = serde_json::to_string(&s1).unwrap();
    // Expected: name1, birthDate1 (primitive) - null fields should be omitted
    let expected1 = r#"{"name1":"Test1","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45}}"#;
    assert_eq!(json1, expected1);

    // Case 2: Only extension for birthDate1
    let s2 = FhirSerdeTestStruct {
        name1: "Test2".to_string().into(), // Required field
        name2: None,
        birth_date1: Date {
            id: Some("bd-id".to_string()),
            extension: None,
            value: None,
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let json2 = serde_json::to_string(&s2).unwrap();
    // Expected: name1, _birthDate1 (object with id)
    let expected2 = r#"{"name1":"Test2","_birthDate1":{"id":"bd-id"},"isActive1":true,"decimal1":123.45,"money1":{"value":123.45}}"#;

    assert_eq!(json2, expected2);

    // Case 3: Both primitive value and extension for birthDate1 and isActive1
    let s3 = FhirSerdeTestStruct {
        name1: "Test3".to_string().into(), // Required field
        name2: None,
        birth_date1: Date {
            id: Some("bd-id-3".to_string()),
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: Boolean {
            id: None,
            extension: None,
            value: Some(true),
        },
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let json3 = serde_json::to_string(&s3).unwrap();
    // Expected: name1, birthDate1 (primitive), _birthDate1 (id), isActive1 (primitive), count
    let expected3 = r#"{"name1":"Test3","birthDate1":"1970-03-30","_birthDate1":{"id":"bd-id-3"},"isActive1":true,"decimal1":123.45,"money1":{"value":123.45}}"#;
    assert_eq!(json3, expected3);

    // Case 4: birthDate1 is default (null), isActive1 has only extension
    let s4 = FhirSerdeTestStruct {
        name1: "Test4".to_string().into(), // Required field
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: Boolean {
            id: None,
            extension: Some(vec![Extension {
                id: None,
                extension: None,
                url: "http://example.com/flag".to_string().into(), // Convert String to Url
                value: Some(ExtensionValue::Boolean(Boolean {
                    id: None,
                    extension: None,
                    value: Some(true),
                })),
            }]),
            value: None,
        },
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let json4 = serde_json::to_string(&s4).unwrap();
    // Expected: name1, _isActive1 (object with extension)
    let expected4 = r#"{"name1":"Test4","birthDate1":"1970-03-30","_isActive1":{"extension":[{"url":"http://example.com/flag","valueBoolean":true}]},"decimal1":123.45,"money1":{"value":123.45}}"#;
    assert_eq!(json4, expected4);

    // Case 5: All optional fields are None/Default
    let s5 = FhirSerdeTestStruct {
        name1: "Test5".to_string().into(), // Required field
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let json5 = serde_json::to_string(&s5).unwrap();
    // Expected: Only required fields (name1, name2) and non-optional elements (birthDate1, isActive1, decimal1, money1) serialized as null
    let expected5 = r#"{"name1":"Test5","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45}}"#;
    assert_eq!(json5, expected5);

    // Case 6: Test Decimal serialization (primitive and extension)
    let s6 = FhirSerdeTestStruct {
        name1: "Test6".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: Decimal {
            id: Some("dec-id".to_string()),
            extension: None,
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(123.45)),
                "123.45".to_string(),
            )),
        },
        decimal2: Some(Decimal {
            // Optional field with only value
            id: None,
            extension: None,
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(98.7)),
                "98.7".to_string(),
            )),
        }),
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let json6 = serde_json::to_string(&s6).unwrap();
    // Expected: decimal1 (primitive), _decimal1 (id), decimal2 (primitive)
    let expected6 = r#"{"name1":"Test6","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"_decimal1":{"id":"dec-id"},"decimal2":98.7,"money1":{"value":123.45}}"#;
    assert_eq!(json6, expected6);

    // Case 7: Test Money serialization (always object)
    let s7 = FhirSerdeTestStruct {
        name1: "Test7".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: Decimal {
            id: None,
            extension: None,
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(123.45)),
                "123.45".to_string(),
            )),
        },
        decimal2: None,
        money1: Money {
            // Required Money field
            id: Some("money-id".to_string().into()), // Convert String to Id
            extension: None,
            // Wrap dec! in PreciseDecimal and DecimalElement
            value: Some(Decimal {
                // This is DecimalElement<Extension>
                id: None,
                extension: None,
                value: Some(PreciseDecimal::from_parts(
                    Some(dec!(100.50)),
                    "100.50".to_string(),
                )),
            }),
            currency: Some(Code {
                // This is Element<String, Extension>
                // Assuming Code is Element<String, Extension>
                id: None,
                extension: None,
                value: Some("USD".to_string()),
            }),
        },
        money2: Some(Money {
            // Optional Money field
            id: None,
            extension: Some(vec![default_extension()]),
            // Wrap dec! in PreciseDecimal and DecimalElement
            value: Some(Decimal {
                // This is DecimalElement<Extension>
                id: None,
                extension: None,
                value: Some(PreciseDecimal::from_parts(
                    Some(dec!(200)),
                    "200".to_string(),
                )),
            }),
            currency: None, // This is Option<Element<String, Extension>>
        }),
        given: None,
    };
    let json7 = serde_json::to_string(&s7).unwrap();
    // Expected: money1 (object), money2 (object)
    // Note: Money always serializes as an object, so no _money1/_money2 split
    let expected7 = r#"{"name1":"Test7","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"id":"money-id","value":100.50,"currency":"USD"},"money2":{"extension":[{"url":"http://example.com/ext","valueString":"ext-val"}],"value":200}}"#;
    assert_eq!(json7, expected7);

    // Case 8: Test Vec<String> serialization (primitive and extension)
    let s8 = FhirSerdeTestStruct {
        name1: "Test8".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: Decimal {
            id: None,
            extension: None,
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(123.45)),
                "123.45".to_string(),
            )),
        },
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: Some(vec![
            String {
                id: None,
                extension: None,
                value: Some("Peter".to_string()),
            }, // Primitive only
            String {
                id: Some("given-id-2".to_string()),
                extension: None,
                value: Some("James".to_string()),
            }, // Value + ID
            String {
                id: None,
                extension: Some(vec![default_extension()]),
                value: None,
            }, // Extension only
            String {
                id: Some("given-id-4".to_string()),
                extension: Some(vec![default_extension()]),
                value: Some("Smith".to_string()),
            }, // Value + ID + Extension
        ]),
    };
    let json8 = serde_json::to_string(&s8).unwrap();

    // Expected: given (array of primitives/nulls), _given (array of objects/nulls for extensions/ids)
    // Note: Keys in the _given objects are sorted alphabetically by serde_json ("extension" before "id").
    let expected8 = r#"{"name1":"Test8","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45},"given":["Peter","James",null,"Smith"],"_given":[null,{"id":"given-id-2"},{"extension":[{"url":"http://example.com/ext","valueString":"ext-val"}]},{"extension":[{"url":"http://example.com/ext","valueString":"ext-val"}],"id":"given-id-4"}]}"#;
    assert_eq!(json8, expected8);

    // Case 9: Test Vec<String> with only primitives
    let s9 = FhirSerdeTestStruct {
        name1: "Test9".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: Decimal {
            id: None,
            extension: None,
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(123.45)),
                "123.45".to_string(),
            )),
        },
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: Some(vec![
            String {
                id: None,
                extension: None,
                value: Some("Alice".to_string()),
            },
            String {
                id: None,
                extension: None,
                value: Some("Bob".to_string()),
            },
        ]),
    };
    let json9 = serde_json::to_string(&s9).unwrap();
    // Expected: Only `given` array, no `_given`
    let expected9 = r#"{"name1":"Test9","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45},"given":["Alice","Bob"]}"#;
    assert_eq!(json9, expected9);

    // Case 10: Test Vec<String> with only extensions/ids
    let s10 = FhirSerdeTestStruct {
        name1: "Test10".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: Decimal {
            id: None,
            extension: None,
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(123.45)),
                "123.45".to_string(),
            )),
        },
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: Some(vec![
            String {
                id: Some("g1".to_string()),
                extension: None,
                value: None,
            },
            String {
                id: None,
                extension: Some(vec![default_extension()]),
                value: None,
            },
        ]),
    };
    let json10 = serde_json::to_string(&s10).unwrap();
    // Expected: `given` array with nulls, `_given` array with objects
    let expected10 = r#"{"name1":"Test10","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45},"_given":[{"id":"g1"},{"extension":[{"url":"http://example.com/ext","valueString":"ext-val"}]}]}"#;
    assert_eq!(json10, expected10);

    // Case 11: Test Vec<String> with null value in primitive array and corresponding extension
    let s11 = FhirSerdeTestStruct {
        name1: "Test11".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: Decimal {
            id: None,
            extension: None,
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(123.45)),
                "123.45".to_string(),
            )),
        },
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: Some(vec![
            String {
                id: None,
                extension: None,
                value: Some("First".to_string()),
            },
            String {
                id: Some("g-null".to_string()),
                extension: None,
                value: None,
            }, // Value is None
            String {
                id: None,
                extension: None,
                value: Some("Last".to_string()),
            },
        ]),
    };
    let json11 = serde_json::to_string(&s11).unwrap();
    // Expected: `given` array with null for the second item, `_given` array with object for the second item's ID
    let expected11 = r#"{"name1":"Test11","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45},"given":["First",null,"Last"],"_given":[null,{"id":"g-null"},null]}"#;
    assert_eq!(json11, expected11);
}

// Test struct with flattened field
#[derive(Debug, PartialEq, FhirSerde, Default)]
struct FlattenTestStruct {
    name: String,

    #[fhir_serde(flatten)]
    nested: NestedStruct,
}

#[derive(Debug, PartialEq, FhirSerde, Default)]
struct NestedStruct {
    field1: String,
    field2: i32,
}

#[test]
fn test_flatten_serialization() {
    // Create a test struct with flattened field
    let test_struct = FlattenTestStruct {
        name: "Test".to_string().into(),
        nested: NestedStruct {
            field1: "Nested".to_string().into(),
            field2: 42,
        },
    };

    // Serialize to JSON
    let json = serde_json::to_string(&test_struct).unwrap();

    // Parse the JSON to verify structure
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();

    // The flattened fields should be at the top level
    assert_eq!(value["name"], "Test");
    assert_eq!(value["field1"], "Nested");
    assert_eq!(value["field2"], 42);

    // There should be no "nested" field
    assert!(value.get("nested").is_none());
}

#[test]
fn test_helios_fhir_serde_deserialize() {
    let decimal = Decimal::new(dec!(123.45));
    // Helper to create default extension
    let default_extension = || Extension {
        id: None,
        extension: None,
        url: "http://example.com/ext".to_string().into(), // Convert String to Url
        value: Some(ExtensionValue::String(String {
            id: None,
            extension: None,
            value: Some("ext-val".to_string()),
        })),
    };

    // Case 1: Only primitive value for birthDate1
    let json1 = r#"{"name1":"Test1","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45}}"#;
    let expected1 = FhirSerdeTestStruct {
        name1: "Test1".to_string().into(), // Required field
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let s1: FhirSerdeTestStruct = serde_json::from_str(json1).unwrap();
    assert_eq!(s1, expected1);

    // Case 2: Only extension for birthDate1
    let json2 = r#"{"name1":"Test2","_birthDate1":{"id":"bd-id","extension":[{"url":"http://example.com/note","valueString":"some note"}]},"isActive1":true,"decimal1":123.45,"money1":{"value":123.45}}"#;
    let expected2 = FhirSerdeTestStruct {
        name1: "Test2".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: Some("bd-id".to_string()),
            extension: Some(vec![Extension {
                id: None,
                extension: None,
                url: "http://example.com/note".to_string().into(), // Convert String to Url
                value: Some(ExtensionValue::String(String {
                    id: None,
                    extension: None,
                    value: Some("some note".to_string()),
                })),
            }]),
            value: None,
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let s2: FhirSerdeTestStruct = serde_json::from_str(json2).unwrap();
    assert_eq!(s2, expected2);

    // Case 3: Both primitive value and extension for birthDate1 and isActive1
    let json3 = r#"{"name1":"Test3","birthDate1":"1970-03-30","_birthDate1":{"id":"bd-id-3","extension":[{"url":"http://example.com/test","valueString":"some-ext-val"}]},"isActive1":true,"_isActive1":{"id":"active-id"},"decimal1":123.45,"money1":{"value":123.45}}"#;
    let expected3 = FhirSerdeTestStruct {
        name1: "Test3".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: Some("bd-id-3".to_string()),
            extension: Some(vec![Extension {
                id: None,
                extension: None,
                url: "http://example.com/test".to_string().into(), // Convert String to Url
                value: Some(ExtensionValue::String(String {
                    id: None,
                    extension: None,
                    value: Some("some-ext-val".to_string()),
                })),
            }]),
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: Boolean {
            id: Some("active-id".to_string()), // Merged from _isActive1
            extension: None,                   // Merged from _isActive1
            value: Some(true),                 // From isActive1
        },
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let s3: FhirSerdeTestStruct = serde_json::from_str(json3).unwrap();
    assert_eq!(s3, expected3);

    // Case 4: isActive1 has only extension
    let json4 = r#"{"name1":"Test4","birthDate1":"1970-03-30","_isActive1":{"extension":[{"url":"http://example.com/flag","valueBoolean":true}]},"decimal1":123.45,"money1":{"value":123.45}}"#;
    let expected4 = FhirSerdeTestStruct {
        name1: "Test4".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: Boolean {
            id: None,
            extension: Some(vec![Extension {
                id: None,
                extension: None,
                url: "http://example.com/flag".to_string().into(), // Convert String to Url
                value: Some(ExtensionValue::Boolean(Boolean {
                    id: None,
                    extension: None,
                    value: Some(true),
                })),
            }]),
            value: None, // isActive1 primitive is missing/null
        },
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let s4: FhirSerdeTestStruct = serde_json::from_str(json4).unwrap();
    assert_eq!(s4, expected4);

    // Case 5: Primitive value is null, but extension exists
    let json5 = r#"{"name1":"Test5","_birthDate1":{"id":"bd-null"},"isActive1":true,"decimal1":123.45,"money1":{"value":123.45}}"#;
    let expected5 = FhirSerdeTestStruct {
        name1: "Test5".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: Some("bd-null".to_string()),
            extension: None,
            value: None, // Value is None because input was null
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let s5: FhirSerdeTestStruct = serde_json::from_str(json5).unwrap();
    assert_eq!(s5, expected5);

    //  No Case 6 or 7

    // Case 8: Duplicate primitive field (should error)
    let json8 = r#"{"birthDate1":"1970-03-30", "birthDate1":"1971-04-01"}"#;
    let res8: Result<FhirSerdeTestStruct, _> = serde_json::from_str(json8);
    assert!(res8.is_err());
    assert!(
        res8.unwrap_err()
            .to_string()
            .contains("duplicate field `birthDate1`")
    );

    // No Case 9

    // Case 10: Deserialize Decimal (primitive and extension)
    let json10 = r#"{"name1":"Test10","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"_decimal1":{"id":"dec-id"},"decimal2":98.7,"money1":{"value":123.45}}"#;
    let expected10 = FhirSerdeTestStruct {
        name1: "Test10".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: Decimal {
            id: Some("dec-id".to_string()),
            extension: None,
            // Deserialization preserves original string rep, value is Some
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(123.45)),
                "123.45".to_string(),
            )),
        },
        decimal2: Some(Decimal {
            id: None,
            extension: None,
            value: Some(PreciseDecimal::from_parts(
                Some(dec!(98.7)),
                "98.7".to_string(),
            )),
        }),
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: None,
    };
    let s10: FhirSerdeTestStruct = serde_json::from_str(json10).unwrap();
    assert_eq!(s10, expected10);

    // Case 11: Deserialize Money (always object)
    let json11 = r#"{"name1":"Test11","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"id":"money-id","value":100.50,"currency":"USD"},"money2":{"extension":[{"url":"http://example.com/ext","valueString":"ext-val"}],"value":200}}"#;
    let expected11 = FhirSerdeTestStruct {
        name1: "Test11".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: Some("money-id".to_string().into()), // Convert String to Id
            extension: None,
            // Wrap dec! in PreciseDecimal and DecimalElement for comparison
            value: Some(Decimal {
                id: None,
                extension: None,
                // Note: Deserialization preserves original string.
                value: Some(PreciseDecimal::from_parts(
                    Some(dec!(100.50)),
                    "100.50".to_string(),
                )),
            }),
            currency: Some(Code {
                value: Some("USD".to_string()),
                ..Default::default()
            }),
        },
        money2: Some(Money {
            id: None,
            extension: Some(vec![default_extension()]),
            // Wrap dec! in PreciseDecimal and DecimalElement for comparison
            value: Some(Decimal {
                id: None,
                extension: None,
                // Assume "200" was the input.
                value: Some(PreciseDecimal::from_parts(
                    Some(dec!(200)),
                    "200".to_string(),
                )),
            }),
            currency: None,
        }),
        given: None,
    };
    let s11: FhirSerdeTestStruct = serde_json::from_str(json11).unwrap();
    assert_eq!(s11, expected11);

    // Case 12: Deserialize Vec<String> (primitive and extension)
    let json12 = r#"{"name1":"Test12","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45},"given":["Peter","James",null,"Smith"],"_given":[null,{"id":"given-id-2"},{"extension":[{"url":"http://example.com/ext","valueString":"ext-val"}]},{"id":"given-id-4","extension":[{"url":"http://example.com/ext","valueString":"ext-val"}]}]}"#;
    let expected12 = FhirSerdeTestStruct {
        name1: "Test12".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: Some(vec![
            String {
                id: None,
                extension: None,
                value: Some("Peter".to_string()),
            },
            String {
                id: Some("given-id-2".to_string()),
                extension: None,
                value: Some("James".to_string()),
            },
            String {
                id: None,
                extension: Some(vec![default_extension()]),
                value: None,
            },
            String {
                id: Some("given-id-4".to_string()),
                extension: Some(vec![default_extension()]),
                value: Some("Smith".to_string()),
            },
        ]),
    };
    let s12: FhirSerdeTestStruct = serde_json::from_str(json12).unwrap();
    assert_eq!(s12, expected12);

    // Case 13: Deserialize Vec<String> with mismatched lengths (should handle gracefully)
    // FHIR spec allows for missing attributes, so we should handle this case
    let json13 = r#"{"name1":"Test13","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45},"given":["Peter"],"_given":[null, {"id":"extra-id"}]}"#;
    let expected13 = FhirSerdeTestStruct {
        name1: "Test13".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: Some(vec![
            String {
                id: None,
                extension: None,
                value: Some("Peter".to_string()),
            },
            // Second element from _given with no corresponding primitive
            String {
                id: Some("extra-id".to_string()),
                extension: None,
                value: None,
            },
        ]),
    };
    let s13: FhirSerdeTestStruct = serde_json::from_str(json13).unwrap();
    assert_eq!(s13, expected13);

    // Case 14: Deserialize Vec<String> with null primitive but non-null extension element
    let json14 = r#"{"name1":"Test14","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45},"_given":[{"id":"g-null"}]}"#;
    let expected14 = FhirSerdeTestStruct {
        name1: "Test14".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: Some(vec![String {
            id: Some("g-null".to_string()),
            extension: None,
            value: None,
        }]),
    };
    let s14: FhirSerdeTestStruct = serde_json::from_str(json14).unwrap();
    assert_eq!(s14, expected14);

    // Case 15: Deserialize Vec<String> with primitive value but null extension element (should be ok)
    let json15 = r#"{"name1":"Test15","birthDate1":"1970-03-30","isActive1":true,"decimal1":123.45,"money1":{"value":123.45},"given":["Value"]}"#;
    let expected15 = FhirSerdeTestStruct {
        name1: "Test15".to_string().into(),
        name2: None,
        birth_date1: Date {
            id: None,
            extension: None,
            value: Some(PrecisionDate::parse("1970-03-30").unwrap()),
        },
        birth_date2: None,
        is_active1: true.into(),
        is_active2: None,
        decimal1: decimal.clone(),
        decimal2: None,
        money1: Money {
            id: None,
            extension: None,
            value: Some(decimal.clone()),
            currency: None,
        },
        money2: None,
        given: Some(vec![String {
            id: None,
            extension: None,
            value: Some("Value".to_string()),
        }]),
    };
    let s15: FhirSerdeTestStruct = serde_json::from_str(json15).unwrap();
    assert_eq!(s15, expected15);
}

#[test]
fn test_timing_roundtrip_serialization() {
    let json_input = r#"
    {
      "timingTiming": {
        "event": [
          null
        ],
        "_event": [
          {
            "extension": [
              {
                "url": "http://hl7.org/fhir/StructureDefinition/cqf-expression",
                "valueExpression": {
                  "language": "text/cql",
                  "expression": "Now()"
                }
              }
            ]
          }
        ]
      }
    }
    "#;

    // Parse original to compare
    let original_value: serde_json::Value = serde_json::from_str(json_input).unwrap();

    // Deserialize the JSON input
    let deserialized_struct: TimingTestStruct =
        serde_json::from_str(json_input).expect("Deserialization failed");

    // Serialize back to JSON
    let reserialized_json =
        serde_json::to_string(&deserialized_struct).expect("Serialization failed");

    // Parse reserialized JSON to compare
    let reserialized_value: serde_json::Value = serde_json::from_str(&reserialized_json).unwrap();

    // The _event field should be preserved in roundtrip
    let original_event = &original_value["timingTiming"]["_event"];
    let reserialized_event = reserialized_value["timingTiming"]
        .get("_event")
        .unwrap_or(&serde_json::Value::Null);
    assert_eq!(
        original_event,
        reserialized_event,
        "The _event field with extensions should be preserved during roundtrip serialization. Original: {}, Reserialized: {}",
        serde_json::to_string_pretty(&original_value).unwrap(),
        serde_json::to_string_pretty(&reserialized_value).unwrap()
    );
}

#[test]
fn test_helios_fhir_serde_deserialize_extension_with_primitive_extension() {
    // This test replicates the structure found in slm-codesystem.json
    // where an Extension has a value[x] (valueString) which itself has an extension (_valueString).
    let json_input = r#"
    {
      "url": "http://hl7.org/fhir/StructureDefinition/codesystem-concept-comments",
      "valueString": "Retained for backwards compatibility only as of v2.6 and CDA R 2. Preferred value is text/xml.",
      "_valueString": {
        "extension": [
          {
            "extension": [
              {
                "url": "lang",
                "valueCode": "nl"
              },
              {
                "url": "content",
                "valueString": "Alleen voor backward compatibiliteit vanaf v2.6 en CDAr2. Voorkeurswaarde is text/xml."
              }
            ],
            "url": "http://hl7.org/fhir/StructureDefinition/translation"
          }
        ]
      }
    }
    "#;

    // Parse the input JSON string into a serde_json::Value for comparison later
    let original_value: serde_json::Value =
        serde_json::from_str(json_input).expect("Parsing original JSON failed");

    // Deserialize the JSON string into the Extension struct
    let extension_struct: Extension =
        serde_json::from_str(json_input).expect("Deserialization into Extension struct failed");

    // Serialize the Extension struct back into a serde_json::Value
    let reserialized_value =
        serde_json::to_value(&extension_struct).expect("Serialization back to JSON value failed");

    // Assert that the reserialized JSON value is identical to the original JSON value
    assert_eq!(
        original_value,
        reserialized_value,
        "Roundtrip failed for Extension with primitive extension.\nOriginal JSON: {}\nReserialized JSON: {}",
        serde_json::to_string_pretty(&original_value).unwrap(),
        serde_json::to_string_pretty(&reserialized_value).unwrap()
    );

    // Explicitly check that the _valueString field exists in the reserialized value
    assert!(
        reserialized_value.get("_valueString").is_some(),
        "_valueString field is missing after roundtrip"
    );
    assert_eq!(
        reserialized_value["_valueString"], original_value["_valueString"],
        "_valueString content mismatch"
    );
}
