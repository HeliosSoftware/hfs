use rust_decimal::Decimal;
use serde::{
    de::{self, Deserializer, Unexpected, MapAccess, DeserializeSeed, Visitor}, // Added Visitor
    ser::{SerializeStruct, Serializer},
    Deserialize, Serialize,
};
use std::marker::PhantomData; // Added PhantomData
//use time::{Date, Month};

#[cfg(feature = "R4")]
pub mod r4;
#[cfg(feature = "R4B")]
pub mod r4b;
#[cfg(feature = "R5")]
pub mod r5;
#[cfg(feature = "R6")]
pub mod r6;

pub trait FhirSerde {
    fn fhir_derive_macro(&self);
}

/// Enum representing a FHIR resource from any supported version
#[derive(Debug)]
pub enum FhirResource {
    #[cfg(feature = "R4")]
    R4(Box<r4::Resource>),
    #[cfg(feature = "R4B")]
    R4B(Box<r4b::Resource>),
    #[cfg(feature = "R5")]
    R5(Box<r5::Resource>),
    #[cfg(feature = "R6")]
    R6(Box<r6::Resource>),
}

impl FhirResource {
    /// Returns the FHIR version of the resource
    pub fn version(&self) -> FhirVersion {
        match self {
            #[cfg(feature = "R4")]
            FhirResource::R4(_) => FhirVersion::R4,
            #[cfg(feature = "R4B")]
            FhirResource::R4B(_) => FhirVersion::R4B,
            #[cfg(feature = "R5")]
            FhirResource::R5(_) => FhirVersion::R5,
            #[cfg(feature = "R6")]
            FhirResource::R6(_) => FhirVersion::R6,
        }
    }
}

/// Represents a FHIR specification version
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FhirVersion {
    #[cfg(feature = "R4")]
    R4,
    #[cfg(feature = "R4B")]
    R4B,
    #[cfg(feature = "R5")]
    R5,
    #[cfg(feature = "R6")]
    R6,
}

impl FhirVersion {
    /// Returns the string representation of the FHIR version
    pub fn as_str(&self) -> &'static str {
        match self {
            #[cfg(feature = "R4")]
            FhirVersion::R4 => "R4",
            #[cfg(feature = "R4B")]
            FhirVersion::R4B => "R4B",
            #[cfg(feature = "R5")]
            FhirVersion::R5 => "R5",
            #[cfg(feature = "R6")]
            FhirVersion::R6 => "R6",
        }
    }
}

impl std::fmt::Display for FhirVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(feature = "R4")]
impl Default for FhirVersion {
    fn default() -> Self {
        FhirVersion::R4
    }
}

// Implement ValueEnum for FhirVersion to support clap
impl clap::ValueEnum for FhirVersion {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            #[cfg(feature = "R4")]
            FhirVersion::R4,
            #[cfg(feature = "R4B")]
            FhirVersion::R4B,
            #[cfg(feature = "R5")]
            FhirVersion::R5,
            #[cfg(feature = "R6")]
            FhirVersion::R6,
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.as_str()))
    }
}

#[derive(Debug)]
pub struct Element<V, E> {
    pub id: Option<String>,
    pub extension: Option<Vec<E>>,
    pub value: Option<V>,
}

impl<'de, V, E> Deserialize<'de> for Element<V, E>
where
    V: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Attempt to deserialize the inner type
        let result = V::deserialize(deserializer);
        match result {
            Ok(value) => Ok(Element {
                id: None,
                extension: None,
                value: Some(value),
            }),
            Err(e) => Err(e),
        }
    }
}

impl<V, E> Serialize for Element<V, E>
where
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.value {
            Some(val) => val.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }
}

#[derive(Debug)]
pub struct DecimalElement<E> {
    pub id: Option<String>,
    pub extension: Option<Vec<E>>,
    pub value: Option<Decimal>,
}

// Visitor is no longer needed with the new Deserialize implementation below

impl<'de, E> Deserialize<'de> for DecimalElement<E>
where
    E: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Intermediate step: Deserialize into serde_json::Value to inspect the type
        let json_value = serde_json::Value::deserialize(deserializer)?;

        match json_value {
            serde_json::Value::Object(map) => {
                // If it's an object, deserialize using the helper struct approach
                // Re-define helper struct here as it's only used in this context now
                #[derive(Deserialize)]
                struct DecimalElementHelper<T> {
                    id: Option<String>,
                    extension: Option<Vec<T>>,
                    #[serde(
                        default, // Use default if 'value' is missing or null in object
                        skip_serializing_if = "Option::is_none",
                        with = "rust_decimal::serde::arbitrary_precision_option"
                    )]
                    value: Option<Decimal>,
                }

                // Create a deserializer from the map's iterator
                let map_deserializer = de::value::MapDeserializer::new(map.into_iter());
                // Deserialize using a visitor designed for the object structure by calling deserialize_map on the deserializer
                map_deserializer.deserialize_map(DecimalObjectVisitor(PhantomData))
            }
            // If it's a number or string, use arbitrary_precision deserializer directly on the Value
            value @ serde_json::Value::Number(_) | value @ serde_json::Value::String(_) => {
                // Deserialize the primitive Value using the precision-preserving deserializer
                let decimal_value =
                    rust_decimal::serde::arbitrary_precision_option::deserialize(value)
                        .map_err(de::Error::custom)?;

                Ok(DecimalElement {
                    id: None,
                    extension: None,
                    value: decimal_value,
                })
            }
            // A bare null is invalid for a required primitive
            serde_json::Value::Null => Err(de::Error::invalid_type(
                Unexpected::Unit, // Use Unexpected::Unit for null
                &"a decimal number, string, or object",
            )),
            // Other types (Array, Bool) are invalid
            other => Err(de::Error::invalid_type(
                other.unexpected(),
                &"a decimal number, string, or object",
            )),
        }
    }
}


// --- Visitor and Seed for Object Deserialization ---

// Seed for deserializing Option<Decimal> with arbitrary precision
struct DecimalOptionSeed;

impl<'de> de::DeserializeSeed<'de> for DecimalOptionSeed {
    type Value = Option<Decimal>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        rust_decimal::serde::arbitrary_precision_option::deserialize(deserializer)
    }
}

// Visitor specifically for deserializing the fields of a DecimalElement from a map
struct DecimalObjectVisitor<E>(PhantomData<E>);

impl<'de, E> Visitor<'de> for DecimalObjectVisitor<E>
where
    E: Deserialize<'de>,
{
    // Use fully qualified syntax for the associated type
    type Value = DecimalElement<E>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a DecimalElement object")
    }

    // Use fully qualified syntax for the return type's associated type
    fn visit_map<A>(self, mut map: A) -> Result<<Self as Visitor<'de>>::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut id: Option<String> = None;
        let mut extension: Option<Vec<E>> = None;
        let mut value: Option<Decimal> = None;

        // Manually deserialize fields from the map
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "id" => {
                    if id.is_some() { return Err(de::Error::duplicate_field("id")); }
                    id = Some(map.next_value()?);
                }
                "extension" => {
                    if extension.is_some() { return Err(de::Error::duplicate_field("extension")); }
                    extension = Some(map.next_value()?);
                }
                "value" => {
                    if value.is_some() { return Err(de::Error::duplicate_field("value")); }
                    // Use the seed to deserialize the value field with arbitrary precision
                    value = map.next_value_seed(DecimalOptionSeed)?;
                }
                // Ignore any unknown fields encountered
                _ => { let _ = map.next_value::<de::IgnoredAny>()?; }
            }
        }

        Ok(DecimalElement { id, extension, value })
    }
}

// --- End Visitor and Seed ---


// Helper extension trait for serde_json::Value to get Unexpected type
trait UnexpectedValue {
    fn unexpected(&self) -> de::Unexpected;
}

impl UnexpectedValue for serde_json::Value {
    fn unexpected(&self) -> Unexpected { // Use the imported Unexpected type directly
        match self {
            serde_json::Value::Null => Unexpected::Unit, // Use Unexpected::Unit for null
            serde_json::Value::Bool(b) => Unexpected::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    Unexpected::Unsigned(u)
                } else if let Some(i) = n.as_i64() {
                    Unexpected::Signed(i)
                } else if let Some(f) = n.as_f64() {
                    Unexpected::Float(f)
                } else {
                    Unexpected::Other("number")
                }
            }
            serde_json::Value::String(s) => Unexpected::Str(s),
            serde_json::Value::Array(_) => Unexpected::Seq,
            serde_json::Value::Object(_) => Unexpected::Map,
        }
    }
}

// --- Serialization Helpers ---

// Helper struct to wrap the Decimal and use the specific serializer
struct SerializeDecimalWithArbitraryPrecision<'a>(&'a Decimal);

impl<'a> Serialize for SerializeDecimalWithArbitraryPrecision<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Call the specific serialize function from the arbitrary_precision module
        rust_decimal::serde::arbitrary_precision::serialize(self.0, serializer)
    }
}

impl<E> Serialize for DecimalElement<E>
where
    E: Serialize, // Add the Serialize bound for the generic type E
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // If we only have a value and no other fields, serialize just the value
        if self.id.is_none() && self.extension.is_none() && self.value.is_some() {
            // Directly serialize the decimal value using our helper wrapper
            return SerializeDecimalWithArbitraryPrecision(self.value.as_ref().unwrap())
                .serialize(serializer);
        }

        // Otherwise, serialize as a struct with all present fields
        // Calculate the number of fields that are NOT None
        let mut len = 0;
        if self.id.is_some() {
            len += 1;
        }
        if self.extension.is_some() {
            len += 1;
        }
        if self.value.is_some() {
            len += 1;
        }

        // Start serializing a struct with the calculated length
        let mut state = serializer.serialize_struct("DecimalElement", len)?;

        // Serialize 'id' field if it's Some
        if let Some(id) = &self.id {
            state.serialize_field("id", id)?;
        }

        // Serialize 'extension' field if it's Some
        if let Some(extension) = &self.extension {
            state.serialize_field("extension", extension)?;
        }

        // Serialize 'value' field if it's Some, using our helper wrapper
        if let Some(value) = &self.value {
            // Pass the inner Decimal to our wrapper struct,
            // which implements Serialize using arbitrary_precision::serialize
            state.serialize_field("value", &SerializeDecimalWithArbitraryPrecision(value))?;
        }

        // End the struct serialization
        state.end()
    }
}

/*
pub trait ElementTrait<V, E> {
    type Value;
    type Extension;

    fn id(&self) -> Option<String>;
    fn set_id(&mut self, id: Option<String>);

    fn extension(&self) -> Option<Vec<Self::Extension>>;
    fn set_extension(&mut self, extension: Option<Vec<Self::Extension>>);

    fn value(&self) -> Option<Self::Value>;
    fn set_value(&mut self, value: Option<Self::Value>);
}

#[derive(Debug)]
pub enum FhirDate {
    /// YYYY
    Year(i32),
    /// YYYY-MM
    YearMonth(i32, Month),
    /// YYYY-MM-DD
    Date(Date),
}
*/

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json;

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
    struct UnitTestExtension {
        code: String,
        is_valid: bool,
    }

    #[test]
    fn test_serialize_decimal_with_value_present() {
        // Use the dec! macro
        let decimal_val = dec!(1050.00);
        let element = DecimalElement::<UnitTestExtension> {
            id: None,
            extension: None,
            value: Some(decimal_val),
        };

        // Serialize the actual element
        let actual_json_string = serde_json::to_string(&element).expect("Serialization failed");
        let actual_value: serde_json::Value =
            serde_json::from_str(&actual_json_string).expect("Parsing actual JSON failed");

        // With our new implementation, a bare decimal with no other fields
        // is serialized as just the number, not an object with a "value" field
        let expected_json_string = r#"1050.00"#;
        let expected_value: serde_json::Value =
            serde_json::from_str(expected_json_string).expect("Parsing expected JSON failed");

        // Compare the parsed serde_json::Value objects
        assert_eq!(
            actual_value, expected_value,
            "Actual JSON: {} \nExpected JSON: {}",
            actual_json_string, expected_json_string
        );
    }

    #[test]
    fn test_serialize_decimal_with_value_absent() {
        let element = DecimalElement::<UnitTestExtension> {
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
        let element = DecimalElement::<UnitTestExtension> {
            id: Some("all-fields-present".to_string()),
            extension: Some(vec![
                UnitTestExtension {
                    code: "C1".to_string(),
                    is_valid: true,
                },
                UnitTestExtension {
                    code: "C2".to_string(),
                    is_valid: false,
                },
            ]),
            value: Some(decimal_val),
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
            Some(&serde_json::json!(-987.654321)),
            "JSON string was: {}",
            json_string
        );
        assert!(json_value.get("extension").is_some());
        assert_eq!(
            json_value["extension"],
            serde_json::json!([
                { "code": "C1", "is_valid": true },
                { "code": "C2", "is_valid": false }
            ])
        );
    }

    #[test]
    fn test_serialize_decimal_with_no_fields() {
        let element = DecimalElement::<UnitTestExtension> {
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
        let element: DecimalElement<UnitTestExtension> =
            serde_json::from_str(json_string).expect("Deserialization failed");

        assert_eq!(element.value, Some(dec!(10)));

        // Test with a bare integer - this needs to be parsed as a JSON value first
        let json_value = serde_json::json!(10);
        let element: DecimalElement<UnitTestExtension> =
            serde_json::from_value(json_value).expect("Deserialization from value failed");

        assert_eq!(element.value, Some(dec!(10)));
    }

    #[test]
    fn test_roundtrip_decimal_serialization() {
        // Test with a bare integer
        let json_value = serde_json::json!(10);

        // Deserialize to our type
        let element: DecimalElement<UnitTestExtension> =
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
        let element: DecimalElement<UnitTestExtension> =
            serde_json::from_value(json_value.clone()).expect("Deserialization failed");

        // Serialize back to JSON
        let reserialized = serde_json::to_value(&element).expect("Serialization failed");

        // Verify we get the same JSON back
        assert_eq!(json_value, reserialized);
    }

    #[test]
    fn test_decimal_with_trailing_zeros() {
        // Test with a decimal value that has trailing zeros (3.0)
        let json_value = serde_json::json!(3.0); // Input is a JSON number 3.0
        let expected_string = "3.0";

        // Deserialize to our type
        let element: DecimalElement<UnitTestExtension> =
            serde_json::from_value(json_value.clone()).expect("Deserialization from number failed");

        // Serialize back to string
        let reserialized_string = serde_json::to_string(&element).expect("Serialization to string failed");

        // Verify the string representation preserves the trailing zero
        assert_eq!(reserialized_string, expected_string,
            "Original JSON Value: {:?}\nExpected String: {}\nReserialized String: {}",
            json_value, expected_string, reserialized_string);

        // Also test with a string representation in the JSON input: "3.0"
        let json_str_input = r#""3.0""#; // Input is a JSON string "3.0"
        // Note: Deserializing a JSON string "3.0" into DecimalElement should still work
        // because the visitor handles visit_str/visit_borrowed_str.
        // The serialized output should still be the bare number 3.0.
        let element_from_string: DecimalElement<UnitTestExtension> =
            serde_json::from_str(json_str_input).expect("Deserialization from string failed");

        // Serialize back to string
        let reserialized_string_from_str = serde_json::to_string(&element_from_string).expect("Serialization to string failed");

        // Verify the string representation is the bare number 3.0
        assert_eq!(reserialized_string_from_str, expected_string,
            "Original JSON String: {}\nExpected String: {}\nReserialized String: {}",
            json_str_input, expected_string, reserialized_string_from_str);

        // Test case from the failure log: parsing the string "3.0" directly
        let json_str = r#"3.0"#; // Input is bare number 3.0 in a string
        let parsed_value: serde_json::Value = serde_json::from_str(json_str).unwrap(); // Parsed as Number(3.0)

        let element_from_bare_string: DecimalElement<UnitTestExtension> =
            serde_json::from_value(parsed_value.clone()).expect("Deserialization from bare string failed");

        let reserialized_string_from_bare = serde_json::to_string(&element_from_bare_string).expect("Serialization failed");

        assert_eq!(reserialized_string_from_bare, expected_string,
            "Original bare string: {}\nParsed Value: {:?}\nExpected String: {}\nReserialized String: {}",
            json_str, parsed_value, expected_string, reserialized_string_from_bare);
    }
}
