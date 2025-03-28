use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{
    de::{self, Deserializer},
    ser::{Serializer, SerializeStruct},
    Deserialize, Serialize,
};
use std::fmt;
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

// Custom serializer implementation
impl<E: Serialize> Serialize for DecimalElement<E> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // If we have id or extension fields, we need to serialize as a complex object
        if self.id.is_some() || self.extension.is_some() {
            let mut state = serializer.serialize_struct("DecimalElement", 3)?;
            if let Some(id) = &self.id {
                state.serialize_field("id", id)?;
            }
            if let Some(ext) = &self.extension {
                state.serialize_field("extension", ext)?;
            }
            if let Some(val) = &self.value {
                state.serialize_field("value", val)?;
            }
            state.end()
        } else {
            // Otherwise, just serialize the decimal value directly
            match &self.value {
                Some(decimal) => decimal.serialize(serializer),
                None => serializer.serialize_none(),
            }
        }
    }
}

// Custom deserializer implementation
impl<'de, E: Deserialize<'de>> Deserialize<'de> for DecimalElement<E> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use a visitor that can handle both direct values and structured data
        struct DecimalElementVisitor<E>(std::marker::PhantomData<E>);

        impl<'de, E: Deserialize<'de>> de::Visitor<'de> for DecimalElementVisitor<E> {
            type Value = DecimalElement<E>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a DecimalElement or a number")
            }

            // Implement visit_map to handle the struct fields
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut id = None;
                let mut extension = None;
                let mut value = None;

                // Process each field in the map
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "id" => {
                            id = Some(map.next_value()?);
                        }
                        "extension" => {
                            extension = Some(map.next_value()?);
                        }
                        "value" => {
                            value = map.next_value::<Option<Decimal>>()?;
                        }
                        _ => {
                            // Skip unknown fields
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(DecimalElement {
                    id,
                    extension,
                    value,
                })
            }

            // Add support for direct numeric values
            fn visit_f64<E2>(self, value: f64) -> Result<Self::Value, E2>
            where
                E2: de::Error,
            {
                match Decimal::try_from(value) {
                    Ok(decimal) => Ok(DecimalElement {
                        id: None,
                        extension: None,
                        value: Some(decimal),
                    }),
                    Err(_) => Err(E2::custom(format!(
                        "Failed to convert f64 {} to Decimal",
                        value
                    ))),
                }
            }

            // Add support for integer values
            fn visit_i64<E2>(self, value: i64) -> Result<Self::Value, E2>
            where
                E2: de::Error,
            {
                match Decimal::from_i64(value) {
                    Some(decimal) => Ok(DecimalElement {
                        id: None,
                        extension: None,
                        value: Some(decimal),
                    }),
                    None => Err(E2::custom(format!(
                        "Failed to convert i64 {} to Decimal",
                        value
                    ))),
                }
            }

            // Add support for unsigned integer values
            fn visit_u64<E2>(self, value: u64) -> Result<Self::Value, E2>
            where
                E2: de::Error,
            {
                match Decimal::from_u64(value) {
                    Some(decimal) => Ok(DecimalElement {
                        id: None,
                        extension: None,
                        value: Some(decimal),
                    }),
                    None => Err(E2::custom(format!(
                        "Failed to convert u64 {} to Decimal",
                        value
                    ))),
                }
            }

            // Add support for string values
            fn visit_str<E2>(self, value: &str) -> Result<Self::Value, E2>
            where
                E2: de::Error,
            {
                match value.parse::<Decimal>() {
                    Ok(decimal) => Ok(DecimalElement {
                        id: None,
                        extension: None,
                        value: Some(decimal),
                    }),
                    Err(_) => Err(E2::custom(format!(
                        "Failed to parse string '{}' as Decimal",
                        value
                    ))),
                }
            }

            // Handle null values
            fn visit_unit<E2>(self) -> Result<Self::Value, E2>
            where
                E2: de::Error,
            {
                Ok(DecimalElement {
                    id: None,
                    extension: None,
                    value: None,
                })
            }
        }

        // Use the visitor to deserialize
        deserializer.deserialize_any(DecimalElementVisitor(std::marker::PhantomData))
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
