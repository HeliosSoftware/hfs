use crate::de::MapAccess;
use crate::de::Visitor;
use rust_decimal::Decimal;
use serde::{
    de::{self, Deserializer},
    ser::{SerializeStruct, Serializer},
    Deserialize, Serialize,
};
use std::fmt;
use std::marker::PhantomData;
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

struct DecimalElementVisitor<E>(PhantomData<E>);

impl<'de, E> DecimalElementVisitor<E>
where
    E: Deserialize<'de>, // Keep constraint here if needed for future helpers
                         // or if the struct itself needs it
{
    // Helper to deserialize a primitive value using the arbitrary_precision_option logic
    fn deserialize_bare_value<D>(&self, deserializer: D) -> Result<Option<Decimal>, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Now this function is correctly called via self.deserialize_bare_value(...)
        rust_decimal::serde::arbitrary_precision_option::deserialize(deserializer)
    }
}

impl<'de, E> Visitor<'de> for DecimalElementVisitor<E>
where
    E: Deserialize<'de>,
{
    type Value = DecimalElement<E>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a decimal value (string or number) or a DecimalElement object")
    }

    // --- Handle the Bare Decimal Case ---

    // Handle number types (integers, floats)
    fn visit_f64<Er>(self, v: f64) -> Result<Self::Value, Er>
    where
        Er: de::Error,
    {
        match Decimal::try_from(v) {
            Ok(decimal) => Ok(DecimalElement {
                id: None,
                extension: None,
                value: Some(decimal),
            }),
            Err(_) => Err(Er::custom(format!("Invalid decimal value: {}", v))),
        }
    }

    fn visit_i64<Er>(self, v: i64) -> Result<Self::Value, Er>
    where
        Er: de::Error,
    {
        Ok(DecimalElement {
            id: None,
            extension: None,
            value: Some(Decimal::from(v)),
        })
    }

    fn visit_u64<Er>(self, v: u64) -> Result<Self::Value, Er>
    where
        Er: de::Error,
    {
        Ok(DecimalElement {
            id: None,
            extension: None,
            value: Some(Decimal::from(v)),
        })
    }

    // Handle string type
    fn visit_str<Er>(self, v: &str) -> Result<Self::Value, Er>
    where
        Er: de::Error,
    {
        let value_deserializer = de::value::StrDeserializer::<'_, Er>::new(v);
        // Call the helper method via self
        let value = self.deserialize_bare_value(value_deserializer)?;
        Ok(DecimalElement {
            id: None,
            extension: None,
            value,
        })
    }

    // Handle borrowed string
    fn visit_borrowed_str<Er>(self, v: &'de str) -> Result<Self::Value, Er>
    where
        Er: de::Error,
    {
        let value_deserializer = de::value::BorrowedStrDeserializer::<'de, Er>::new(v);
        // Call the helper method via self
        let value = self.deserialize_bare_value(value_deserializer)?;
        Ok(DecimalElement {
            id: None,
            extension: None,
            value,
        })
    }

    // --- Handle the JSON Object Case ---

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        // Use a helper struct for deserializing the object structure
        #[derive(Deserialize)]
        struct DecimalElementHelper<T> {
            id: Option<String>,
            extension: Option<Vec<T>>,
            // Use with = instead of deserialize_with to avoid the Clone constraint issue
            #[serde(
                skip_serializing_if = "Option::is_none",
                with = "decimal_option_deserializer"
            )]
            value: Option<Decimal>,
        }

        // Module to handle decimal deserialization without requiring Clone
        mod decimal_option_deserializer {
            use super::*;
            
            pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Decimal>, D::Error>
            where
                D: Deserializer<'de>,
            {
                // Custom visitor that handles all number formats
                struct DecimalOptionVisitor;
                
                impl<'de> Visitor<'de> for DecimalOptionVisitor {
                    type Value = Option<Decimal>;
                    
                    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                        formatter.write_str("a decimal value as number or string")
                    }
                    
                    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        Ok(Some(Decimal::from(v)))
                    }
                    
                    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        Ok(Some(Decimal::from(v)))
                    }
                    
                    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        match Decimal::try_from(v) {
                            Ok(d) => Ok(Some(d)),
                            Err(_) => Err(E::custom(format!("Invalid decimal value: {}", v))),
                        }
                    }
                    
                    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        // Try to parse the string as a decimal
                        match v.parse::<Decimal>() {
                            Ok(d) => Ok(Some(d)),
                            Err(_) => Err(E::custom(format!("Invalid decimal string: {}", v))),
                        }
                    }
                    
                    fn visit_none<E>(self) -> Result<Self::Value, E>
                    where
                        E: de::Error,
                    {
                        Ok(None)
                    }
                    
                    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
                    where
                        D: Deserializer<'de>,
                    {
                        deserializer.deserialize_any(self)
                    }
                }
                
                // Try to deserialize using our custom visitor
                deserializer.deserialize_any(DecimalOptionVisitor)
            }
            
            // We need to implement serialize too for the with attribute
            pub fn serialize<S>(value: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                match value {
                    Some(decimal) => {
                        rust_decimal::serde::arbitrary_precision::serialize(decimal, serializer)
                    }
                    None => serializer.serialize_none(),
                }
            }
        }

        // Deserialize the map into the helper struct
        let helper: DecimalElementHelper<E> =
            Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))?;

        // Construct the actual DecimalElement from the helper
        Ok(DecimalElement {
            id: helper.id,
            extension: helper.extension,
            value: helper.value,
        })
    }
}

impl<'de, E> Deserialize<'de> for DecimalElement<E>
where
    E: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use deserialize_any to handle different top-level JSON types
        deserializer.deserialize_any(DecimalElementVisitor(PhantomData))
    }
}

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
            // which implements Serialize using string representation
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
        assert_eq!(json_value, reserialized, 
            "Original: {:?}\nReserialized: {:?}", 
            json_value, reserialized);
        
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
