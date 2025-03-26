use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
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
            FhirVersion::R4 => "r4",
            #[cfg(feature = "R4B")]
            FhirVersion::R4B => "r4b",
            #[cfg(feature = "R5")]
            FhirVersion::R5 => "r5",
            #[cfg(feature = "R6")]
            FhirVersion::R6 => "r6",
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

// Generic implementation of Deserialize
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

// Generic implementation of Serialize
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

/// A decimal type that preserves exact precision when serializing to/from JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct FhirDecimal {
    pub value: Decimal,
    pub scale: usize, // Explicit scale (number of decimal places)
}

impl FhirDecimal {
    /// Create a new FhirDecimal from an f64 value
    pub fn new(value: f64) -> Result<Self, rust_decimal::Error> {
        let decimal = Decimal::from_str(&value.to_string())?;
        let scale = determine_scale(value);
        // AI! print the decimal and scale
        Ok(FhirDecimal {
            value: decimal,
            scale,
        })
    }

    /// Create from a Decimal with specified scale
    pub fn with_scale(value: Decimal, scale: usize) -> Self {
        FhirDecimal { value, scale }
    }

    /// Get the underlying Decimal value
    pub fn value(&self) -> Decimal {
        self.value
    }

    /// Convert to f64
    pub fn to_f64(&self) -> f64 {
        self.value.to_f64().unwrap_or(0.0)
    }
}

// Determine scale from f64
fn determine_scale(value: f64) -> usize {
    let s = value.to_string();
    if let Some(dot_pos) = s.find('.') {
        s.len() - dot_pos - 1
    } else {
        0
    }
}

// Custom implementation of Serialize
impl Serialize for FhirDecimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Always respect the original scale, even for whole numbers
        // Only serialize as integer if scale is explicitly 0
        if self.scale == 0 && self.value.fract().is_zero() {
            if let Some(int_val) = self.value.to_i64() {
                return serializer.serialize_i64(int_val);
            }
        }

        // Format with exact scale, preserving decimal places
        let formatted = format!("{:.*}", self.scale, self.value);
        serializer.serialize_str(&formatted)
    }
}

// Custom implementation of Deserialize
impl<'de> Deserialize<'de> for FhirDecimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum NumericOrString {
            String(String),
            Int(i64),
            Float(f64),
        }

        let value = NumericOrString::deserialize(deserializer)?;

        match value {
            NumericOrString::String(s) => {
                let decimal = Decimal::from_str(&s).map_err(serde::de::Error::custom)?;

                // Calculate scale from the string representation
                let scale = if let Some(dot_pos) = s.find('.') {
                    s.len() - dot_pos - 1
                } else {
                    0
                };

                Ok(FhirDecimal {
                    value: decimal,
                    scale,
                })
            }
            NumericOrString::Int(i) => {
                let decimal = Decimal::from(i);
                // For integers, set scale to 0
                Ok(FhirDecimal {
                    value: decimal,
                    scale: 0,
                })
            }
            NumericOrString::Float(f) => {
                let decimal =
                    Decimal::from_str(&f.to_string()).map_err(serde::de::Error::custom)?;

                // For floats, determine scale from the string representation
                let scale = determine_scale(f);

                Ok(FhirDecimal {
                    value: decimal,
                    scale,
                })
            }
        }
    }
}

// Implement Display
impl fmt::Display for FhirDecimal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // If scale is 0 and it's a whole number, format as integer
        if self.scale == 0 && self.value.fract().is_zero() {
            if let Some(int_val) = self.value.to_i64() {
                return write!(f, "{}", int_val);
            }
        }

        // Otherwise format with the specified scale
        write!(f, "{:.*}", self.scale, self.value)
    }
}

// Convert from common types
impl From<i32> for FhirDecimal {
    fn from(value: i32) -> Self {
        FhirDecimal {
            value: Decimal::from(value),
            scale: 0,
        }
    }
}

impl From<i64> for FhirDecimal {
    fn from(value: i64) -> Self {
        FhirDecimal {
            value: Decimal::from(value),
            scale: 0,
        }
    }
}

// Create with specific scale - fixed implementation
impl FhirDecimal {
    pub fn from_with_scale(value: i64, scale: usize) -> Self {
        // Use string manipulation to ensure correct decimal places
        let value_str = if scale > 0 {
            format!("{}.{}", value, "0".repeat(scale))
        } else {
            value.to_string()
        };

        // Parse the formatted string
        let decimal = Decimal::from_str(&value_str).unwrap_or(Decimal::from(value));
        FhirDecimal {
            value: decimal,
            scale,
        }
    }
}

impl TryFrom<f64> for FhirDecimal {
    type Error = rust_decimal::Error;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        FhirDecimal::new(value)
    }
}

impl TryFrom<&str> for FhirDecimal {
    type Error = rust_decimal::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let decimal = Decimal::from_str(value)?;

        // Calculate scale from the string
        let scale = if let Some(dot_pos) = value.find('.') {
            value.len() - dot_pos - 1
        } else {
            0
        };

        Ok(FhirDecimal {
            value: decimal,
            scale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_serialize_integer() {
        let decimal = FhirDecimal::from(1250);
        let json = serde_json::to_string(&decimal).unwrap();
        assert_eq!(json, "1250");
    }

    #[test]
    fn test_serialize_decimal() {
        // Create a decimal with value 250 and scale 1 (should be 250.0)
        let decimal = FhirDecimal::with_scale(Decimal::from_str("250.0").unwrap(), 1);

        // Verify the display shows 250.0
        assert_eq!(format!("{}", decimal), "250.0");

        // Now check serialization
        let json = serde_json::to_string(&decimal).unwrap();
        assert_eq!(json, "\"250.0\"");
    }

    #[test]
    fn test_roundtrip() {
        // Create a decimal with value 250 and scale 1 (should be 250.0)
        let original = FhirDecimal::with_scale(Decimal::from_str("250.0").unwrap(), 1);

        // Verify original display
        assert_eq!(format!("{}", original), "250.0");

        // Serialize
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"250.0\"");

        // Deserialize
        let roundtrip: FhirDecimal = serde_json::from_str(&json).unwrap();

        // Verify value and scale
        assert_eq!(original.value, roundtrip.value);
        assert_eq!(original.scale, roundtrip.scale);

        // Verify display format
        assert_eq!(format!("{}", roundtrip), "250.0");
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
