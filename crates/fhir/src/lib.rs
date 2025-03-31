use rust_decimal::Decimal;
use serde::{
    de::{self, Deserializer, MapAccess, Visitor}, // Removed Unexpected
    ser::{SerializeStruct, Serializer},           // Added SerializeStruct
    Deserialize,
    Serialize,
};
use std::marker::PhantomData; // Re-added PhantomData
                              // Removed unused RawValue import
use std::ops::{Deref, DerefMut}; // Needed for Newtype pattern convenience
                                 //use time::{Date, Month};

// --- Newtype wrapper for precise Decimal serialization ---

// Add Eq derive
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PreciseDecimal(pub Decimal); // Make inner Decimal public for convenience

// Allow accessing the inner Decimal easily
impl Deref for PreciseDecimal {
    type Target = Decimal;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for PreciseDecimal {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Serialize for PreciseDecimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize using rust_decimal's arbitrary precision serializer.
        // This outputs a JSON number. serde_json's to_string representation
        // seems to preserve trailing zeros correctly with this serializer.
        rust_decimal::serde::arbitrary_precision::serialize(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for PreciseDecimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize into an intermediate serde_json::Value first
        let json_value = serde_json::Value::deserialize(deserializer)?;

        match json_value {
            serde_json::Value::String(s) => {
                // If it's a string, parse it directly
                s.parse::<Decimal>()
                    .map(PreciseDecimal)
                    .map_err(de::Error::custom)
            }
            serde_json::Value::Number(n) => {
                // If it's a number, convert it to a string first, then parse.
                // This preserves the scale (e.g., 3.0 stays "3.0").
                n.to_string()
                    .parse::<Decimal>()
                    .map(PreciseDecimal)
                    .map_err(de::Error::custom)
            }
            other => {
                // Handle other unexpected types
                Err(de::Error::invalid_type(
                    match other {
                        serde_json::Value::Null => de::Unexpected::Unit,
                        serde_json::Value::Bool(b) => de::Unexpected::Bool(b),
                        serde_json::Value::Array(_) => de::Unexpected::Seq,
                        serde_json::Value::Object(_) => de::Unexpected::Map,
                        // Should not happen based on Value enum definition
                        _ => de::Unexpected::Other("unexpected JSON type"),
                    },
                    &"a number or a string",
                ))
            }
        }
    }
}

// --- End Newtype ---

// --- Visitor for Object Deserialization ---

// Visitor specifically for deserializing the fields of a DecimalElement from a map
struct DecimalElementObjectVisitor<E>(PhantomData<E>);

impl<'de, E> Visitor<'de> for DecimalElementObjectVisitor<E>
where
    E: Deserialize<'de>,
{
    type Value = DecimalElement<E>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a DecimalElement object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut id: Option<String> = None;
        let mut extension: Option<Vec<E>> = None;
        // Expect PreciseDecimal now
        let mut value: Option<PreciseDecimal> = None;

        // Manually deserialize fields from the map
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "id" => {
                    if id.is_some() {
                        return Err(de::Error::duplicate_field("id"));
                    }
                    id = Some(map.next_value()?);
                }
                "extension" => {
                    if extension.is_some() {
                        return Err(de::Error::duplicate_field("extension"));
                    }
                    extension = Some(map.next_value()?);
                }
                "value" => {
                    if value.is_some() {
                        return Err(de::Error::duplicate_field("value"));
                    }
                    // Deserialize directly into Option<PreciseDecimal>
                    value = map.next_value()?;
                }
                // Ignore any unknown fields encountered
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }

        Ok(DecimalElement {
            id,
            extension,
            value,
        })
    }
}

// --- End Visitor ---

#[cfg(feature = "R4")]
pub mod r4;
#[cfg(feature = "R4B")]
pub mod r4b;
#[cfg(feature = "R5")]
pub mod r5;
#[cfg(feature = "R6")]
pub mod r6;

// Removed the FhirSerde trait definition

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

// --- Visitor for Element Object Deserialization ---
struct ElementObjectVisitor<V, E>(PhantomData<(V, E)>);

impl<'de, V, E> Visitor<'de> for ElementObjectVisitor<V, E>
where
    V: Deserialize<'de>,
    E: Deserialize<'de>,
{
    type Value = Element<V, E>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an Element object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut id: Option<String> = None;
        let mut extension: Option<Vec<E>> = None;
        let mut value: Option<V> = None;

        // Manually deserialize fields from the map
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "id" => {
                    if id.is_some() {
                        return Err(de::Error::duplicate_field("id"));
                    }
                    id = Some(map.next_value()?);
                }
                "extension" => {
                    if extension.is_some() {
                        return Err(de::Error::duplicate_field("extension"));
                    }
                    extension = Some(map.next_value()?);
                }
                "value" => {
                    if value.is_some() {
                        return Err(de::Error::duplicate_field("value"));
                    }
                    // Deserialize directly into Option<V>
                    value = Some(map.next_value()?);
                }
                // Ignore any unknown fields encountered
                _ => {
                    let _ = map.next_value::<de::IgnoredAny>()?;
                }
            }
        }

        Ok(Element {
            id,
            extension,
            value,
        })
    }
}
// --- End Element Visitor ---

// Add PartialEq, Eq, Clone, Copy derives where applicable
// Note: Cannot add Copy if V or E are not Copy (like String, Vec)
// Add Clone derive
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Element<V, E> {
    // Fields are already public
    pub id: Option<String>,
    pub extension: Option<Vec<E>>,
    pub value: Option<V>,
}

// Custom Deserialize for Element<V, E>
// Remove PartialEq/Eq bounds for V and E as they are not needed for deserialization itself
impl<'de, V, E> Deserialize<'de> for Element<V, E>
where
    V: Deserialize<'de>, // Removed PartialEq + Eq
    E: Deserialize<'de>, // Removed PartialEq
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use the AnyValueVisitor approach to handle different JSON input types
        struct AnyValueVisitor<V, E>(PhantomData<(V, E)>);

        impl<'de, V, E> Visitor<'de> for AnyValueVisitor<V, E>
        where
            V: Deserialize<'de>,
            E: Deserialize<'de>,
        {
            type Value = Element<V, E>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter
                    .write_str("a primitive value (string, number, boolean), an object, or null")
            }

            // Handle primitive types by attempting to deserialize V and wrapping it
            fn visit_bool<Er>(self, v: bool) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                V::deserialize(de::value::BoolDeserializer::new(v))
                    .map(|value| Element {
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }
            fn visit_i64<Er>(self, v: i64) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                V::deserialize(de::value::I64Deserializer::new(v))
                    .map(|value| Element {
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }
            fn visit_u64<Er>(self, v: u64) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                V::deserialize(de::value::U64Deserializer::new(v))
                    .map(|value| Element {
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }
            fn visit_f64<Er>(self, v: f64) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                V::deserialize(de::value::F64Deserializer::new(v))
                    .map(|value| Element {
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }
            fn visit_str<Er>(self, v: &str) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                V::deserialize(de::value::StrDeserializer::new(v))
                    .map(|value| Element {
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }
            fn visit_string<Er>(self, v: String) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                V::deserialize(de::value::StringDeserializer::new(v.clone()))
                    .map(|value| Element {
                        // Clone v for error message
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }
            fn visit_borrowed_str<Er>(self, v: &'de str) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                V::deserialize(de::value::BorrowedStrDeserializer::new(v))
                    .map(|value| Element {
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }
            fn visit_bytes<Er>(self, v: &[u8]) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                V::deserialize(de::value::BytesDeserializer::new(v))
                    .map(|value| Element {
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }
            fn visit_byte_buf<Er>(self, v: Vec<u8>) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                // Use BytesDeserializer with a slice reference &v
                V::deserialize(de::value::BytesDeserializer::new(&v))
                    .map(|value| Element {
                        id: None,
                        extension: None,
                        value: Some(value),
                    })
                    // Propagate the error from V::deserialize directly
                    .map_err(|e| e)
            }

            // Handle null
            fn visit_none<Er>(self) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                Ok(Element {
                    id: None,
                    extension: None,
                    value: None,
                })
            }
            fn visit_unit<Er>(self) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                Ok(Element {
                    id: None,
                    extension: None,
                    value: None,
                })
            }

            // Handle Option<T> by visiting Some
            fn visit_some<De>(self, deserializer: De) -> Result<Self::Value, De::Error>
            where
                De: Deserializer<'de>,
            {
                // Re-dispatch to deserialize_any to handle the inner type correctly
                deserializer.deserialize_any(self)
            }

            // Handle object
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                // Deserialize the map using ElementObjectVisitor
                // Need to create a deserializer from the map access
                let map_deserializer = de::value::MapAccessDeserializer::new(map);
                map_deserializer.deserialize_map(ElementObjectVisitor(PhantomData))
            }

            // We don't expect sequences for a single Element
            fn visit_seq<A>(self, _seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                Err(de::Error::invalid_type(de::Unexpected::Seq, &self))
            }
        }

        // Start deserialization using the visitor
        deserializer.deserialize_any(AnyValueVisitor(PhantomData))
    }
}

// Custom Serialize for Element<V, E>
// Remove PartialEq/Eq bounds for V and E as they are not needed for serialization itself
impl<V, E> Serialize for Element<V, E>
where
    V: Serialize, // Removed PartialEq + Eq
    E: Serialize, // Removed PartialEq
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // If id and extension are None, serialize value directly (or null)
        if self.id.is_none() && self.extension.is_none() {
            match &self.value {
                Some(val) => val.serialize(serializer),
                None => serializer.serialize_none(),
            }
        } else {
            // Otherwise, serialize as an object containing id, extension, value if present
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

            let mut state = serializer.serialize_struct("Element", len)?;
            if let Some(id) = &self.id {
                state.serialize_field("id", id)?;
            }
            if let Some(extension) = &self.extension {
                state.serialize_field("extension", extension)?;
            }
            if let Some(value) = &self.value {
                // Important: Serialize the value under the "value" key when part of the object
                state.serialize_field("value", value)?;
            }
            state.end()
        }
    }
}

// Add Clone derive
#[derive(Debug, PartialEq, Eq, Clone)]
// Remove serde attributes as they are not used without derive
pub struct DecimalElement<E> {
    pub id: Option<String>,
    pub extension: Option<Vec<E>>,
    // Use the PreciseDecimal wrapper for the value field
    pub value: Option<PreciseDecimal>,
}

// Reinstate custom Deserialize implementation
// Remove PartialEq bound for E
impl<'de, E> Deserialize<'de> for DecimalElement<E>
where
    E: Deserialize<'de>, // Removed PartialEq bound for E
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Intermediate step: Deserialize into serde_json::Value to inspect the type
        // We need a visitor that can handle any JSON type first
        struct AnyValueVisitor;

        impl<'de> Visitor<'de> for AnyValueVisitor {
            type Value = serde_json::Value;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("any valid JSON value")
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                // Deserialize the map into a serde_json::Map
                let map_value: serde_json::Map<String, serde_json::Value> =
                    Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(serde_json::Value::Object(map_value))
            }

            fn visit_bool<Er>(self, v: bool) -> Result<Self::Value, Er> {
                Ok(serde_json::Value::Bool(v))
            }
            fn visit_i64<Er>(self, v: i64) -> Result<Self::Value, Er> {
                Ok(serde_json::Number::from(v).into())
            }
            fn visit_u64<Er>(self, v: u64) -> Result<Self::Value, Er> {
                Ok(serde_json::Number::from(v).into())
            }
            fn visit_f64<Er>(self, v: f64) -> Result<Self::Value, Er> {
                Ok(serde_json::Number::from_f64(v)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)) // Handle non-finite floats
            }
            fn visit_str<Er>(self, v: &str) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                Ok(serde_json::Value::String(v.to_owned()))
            }
            fn visit_string<Er>(self, v: String) -> Result<Self::Value, Er> {
                Ok(serde_json::Value::String(v))
            }
            fn visit_borrowed_str<Er>(self, v: &'de str) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                Ok(serde_json::Value::String(v.to_owned()))
            }
            fn visit_bytes<Er>(self, v: &[u8]) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                Ok(serde_json::Value::String(
                    String::from_utf8_lossy(v).into_owned(),
                ))
            }
            fn visit_byte_buf<Er>(self, v: Vec<u8>) -> Result<Self::Value, Er>
            where
                Er: de::Error,
            {
                Ok(serde_json::Value::String(
                    String::from_utf8_lossy(&v).into_owned(),
                ))
            }
            fn visit_none<Er>(self) -> Result<Self::Value, Er> {
                Ok(serde_json::Value::Null)
            }
            fn visit_some<De>(self, deserializer: De) -> Result<Self::Value, De::Error>
            where
                De: Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }
            fn visit_unit<Er>(self) -> Result<Self::Value, Er> {
                Ok(serde_json::Value::Null)
            }
            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let vec: Vec<serde_json::Value> =
                    Deserialize::deserialize(de::value::SeqAccessDeserializer::new(seq))?;
                Ok(serde_json::Value::Array(vec))
            }
        }

        let json_value = deserializer.deserialize_any(AnyValueVisitor)?;

        match json_value {
            serde_json::Value::Object(map) => {
                // If it's an object, deserialize using the DecimalElementObjectVisitor
                let map_deserializer = de::value::MapDeserializer::new(map.into_iter());
                map_deserializer
                    .deserialize_map(DecimalElementObjectVisitor(PhantomData))
                    .map_err(de::Error::custom)
            }
            // If it's a number or string, deserialize directly into PreciseDecimal
            value @ serde_json::Value::Number(_) | value @ serde_json::Value::String(_) => {
                // Deserialize the primitive Value using PreciseDecimal's Deserialize impl
                let precise_decimal =
                    PreciseDecimal::deserialize(value).map_err(de::Error::custom)?;

                Ok(DecimalElement {
                    id: None,
                    extension: None,
                    value: Some(precise_decimal),
                })
            }
            // A bare null might represent an element with no value
            serde_json::Value::Null => Ok(DecimalElement {
                id: None,
                extension: None,
                value: None,
            }),
            // Other types (Array, Bool) are invalid for representing a DecimalElement
            other => Err(de::Error::invalid_type(
                // Need UnexpectedValue helper or similar logic here
                // Let's use a simple description for now
                match other {
                    serde_json::Value::Array(_) => de::Unexpected::Seq,
                    serde_json::Value::Bool(b) => de::Unexpected::Bool(b),
                    _ => de::Unexpected::Other("unexpected JSON type"),
                },
                &"a decimal number, string, object, or null",
            )),
        }
    }
}

// Note: The custom Deserialize impl below handles both object and primitive cases.

// Helper extension trait for serde_json::Value to get Unexpected type
// Used in the custom Deserialize implementation below.
/* // Trait is unused currently
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
*/ // Trait is unused currently

// Reinstate custom Serialize implementation for DecimalElement
// Remove PartialEq bound for E
impl<E> Serialize for DecimalElement<E>
where
    E: Serialize, // Removed PartialEq bound for E
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // If we only have a value and no other fields, serialize just the value
        if self.id.is_none() && self.extension.is_none() {
            if let Some(value) = &self.value {
                // Serialize the PreciseDecimal directly, invoking its custom Serialize impl
                return value.serialize(serializer);
            } else {
                // If value is also None, serialize as an empty object
                // based on test_serialize_decimal_with_no_fields
                return serializer.serialize_struct("DecimalElement", 0)?.end();
            }
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

        // Serialize 'value' field if it's Some
        if let Some(value) = &self.value {
            // Serialize the PreciseDecimal directly, invoking its custom Serialize impl
            state.serialize_field("value", value)?;
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
    // Keep existing imports
    use super::*;
    // Re-add FhirSerde import as the test struct uses the derive macro again
    use fhir_macro::FhirSerde;
    use rust_decimal_macros::dec;
    use serde_json;

    // Add Eq, Default derives
    // Add Serialize derive back since it's used in tests directly now
    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
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
            // Wrap the Decimal in PreciseDecimal
            value: Some(PreciseDecimal(decimal_val)),
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
            // Wrap the Decimal in PreciseDecimal
            value: Some(PreciseDecimal(decimal_val)),
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

        // Compare against PreciseDecimal
        assert_eq!(element.value, Some(PreciseDecimal(dec!(10))));

        // Test with a bare integer - this needs to be parsed as a JSON value first
        let json_value = serde_json::json!(10);
        let element: DecimalElement<UnitTestExtension> =
            serde_json::from_value(json_value).expect("Deserialization from value failed");

        // Compare against PreciseDecimal
        assert_eq!(element.value, Some(PreciseDecimal(dec!(10))));
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
        let element: DecimalElement<UnitTestExtension> =
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
        let element_from_string: DecimalElement<UnitTestExtension> =
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

        let element_from_bare_string: DecimalElement<UnitTestExtension> =
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
    }

    // --- New tests for Element<V, E> ---

    #[test]
    fn test_serialize_element_primitive() {
        let element = Element::<String, UnitTestExtension> {
            id: None,
            extension: None,
            value: Some("test_value".to_string()),
        };
        let json_string = serde_json::to_string(&element).unwrap();
        // Should serialize as the primitive value directly
        assert_eq!(json_string, r#""test_value""#);

        let element_null = Element::<String, UnitTestExtension> {
            id: None,
            extension: None,
            value: None,
        };
        let json_string_null = serde_json::to_string(&element_null).unwrap();
        // Should serialize as null
        assert_eq!(json_string_null, "null");

        // Test with integer
        let element_int = Element::<i32, UnitTestExtension> {
            id: None,
            extension: None,
            value: Some(123),
        };
        let json_string_int = serde_json::to_string(&element_int).unwrap();
        assert_eq!(json_string_int, "123");

        // Test with boolean
        let element_bool = Element::<bool, UnitTestExtension> {
            id: None,
            extension: None,
            value: Some(true),
        };
        let json_string_bool = serde_json::to_string(&element_bool).unwrap();
        assert_eq!(json_string_bool, "true");
    }

    #[test]
    fn test_serialize_element_object() {
        let element = Element::<String, UnitTestExtension> {
            id: Some("elem-id".to_string()),
            extension: Some(vec![UnitTestExtension {
                code: "ext1".to_string(),
                is_valid: true,
            }]),
            value: Some("test_value".to_string()),
        };
        let json_string = serde_json::to_string(&element).unwrap();
        // Should serialize as an object because id/extension are present
        let expected_json = r#"{"id":"elem-id","extension":[{"code":"ext1","is_valid":true}],"value":"test_value"}"#;
        assert_eq!(json_string, expected_json);

        // Test with only id
        let element_id_only = Element::<String, UnitTestExtension> {
            id: Some("elem-id-only".to_string()),
            extension: None,
            value: Some("test_value_id".to_string()),
        };
        let json_string_id_only = serde_json::to_string(&element_id_only).unwrap();
        let expected_json_id_only = r#"{"id":"elem-id-only","value":"test_value_id"}"#;
        assert_eq!(json_string_id_only, expected_json_id_only);

        // Test with only extension
        let element_ext_only = Element::<String, UnitTestExtension> {
            id: None,
            extension: Some(vec![UnitTestExtension {
                code: "ext2".to_string(),
                is_valid: false,
            }]),
            value: Some("test_value_ext".to_string()),
        };
        let json_string_ext_only = serde_json::to_string(&element_ext_only).unwrap();
        let expected_json_ext_only =
            r#"{"extension":[{"code":"ext2","is_valid":false}],"value":"test_value_ext"}"#;
        assert_eq!(json_string_ext_only, expected_json_ext_only);

        // Test with id, extension, but no value
        let element_no_value = Element::<String, UnitTestExtension> {
            id: Some("elem-id-no-val".to_string()),
            extension: Some(vec![UnitTestExtension {
                code: "ext3".to_string(),
                is_valid: true,
            }]),
            value: None,
        };
        let json_string_no_value = serde_json::to_string(&element_no_value).unwrap();
        // Should serialize object without the "value" field
        let expected_json_no_value =
            r#"{"id":"elem-id-no-val","extension":[{"code":"ext3","is_valid":true}]}"#;
        assert_eq!(json_string_no_value, expected_json_no_value);
    }

    #[test]
    fn test_deserialize_element_primitive() {
        // String primitive
        let json_string = r#""test_value""#;
        let element: Element<String, UnitTestExtension> =
            serde_json::from_str(json_string).unwrap();
        assert_eq!(element.id, None);
        assert_eq!(element.extension, None);
        assert_eq!(element.value, Some("test_value".to_string()));

        // Null primitive
        let json_null = "null";
        let element_null: Element<String, UnitTestExtension> =
            serde_json::from_str(json_null).unwrap();
        assert_eq!(element_null.id, None);
        assert_eq!(element_null.extension, None);
        assert_eq!(element_null.value, None);

        // Number primitive
        let json_num = "123";
        let element_num: Element<i32, UnitTestExtension> = serde_json::from_str(json_num).unwrap();
        assert_eq!(element_num.id, None);
        assert_eq!(element_num.extension, None);
        assert_eq!(element_num.value, Some(123));

        // Boolean primitive
        let json_bool = "true";
        let element_bool: Element<bool, UnitTestExtension> =
            serde_json::from_str(json_bool).unwrap();
        assert_eq!(element_bool.id, None);
        assert_eq!(element_bool.extension, None);
        assert_eq!(element_bool.value, Some(true));
    }

    #[test]
    fn test_deserialize_element_object() {
        // Full object
        let json_string = r#"{"id":"elem-id","extension":[{"code":"ext1","is_valid":true}],"value":"test_value"}"#;
        let element: Element<String, UnitTestExtension> =
            serde_json::from_str(json_string).unwrap();
        assert_eq!(element.id, Some("elem-id".to_string()));
        assert_eq!(
            element.extension,
            Some(vec![UnitTestExtension {
                code: "ext1".to_string(),
                is_valid: true
            }])
        );
        assert_eq!(element.value, Some("test_value".to_string()));

        // Object with missing value
        let json_missing_value =
            r#"{"id":"elem-id-no-val","extension":[{"code":"ext3","is_valid":true}]}"#;
        let element_missing_value: Element<String, UnitTestExtension> =
            serde_json::from_str(json_missing_value).unwrap();
        assert_eq!(element_missing_value.id, Some("elem-id-no-val".to_string()));
        assert_eq!(
            element_missing_value.extension,
            Some(vec![UnitTestExtension {
                code: "ext3".to_string(),
                is_valid: true
            }])
        );
        assert_eq!(element_missing_value.value, None); // Value should be None

        // Object with missing extension
        let json_missing_ext = r#"{"id":"elem-id-only","value":"test_value_id"}"#;
        let element_missing_ext: Element<String, UnitTestExtension> =
            serde_json::from_str(json_missing_ext).unwrap();
        assert_eq!(element_missing_ext.id, Some("elem-id-only".to_string()));
        assert_eq!(element_missing_ext.extension, None);
        assert_eq!(element_missing_ext.value, Some("test_value_id".to_string()));

        // Object with missing id
        let json_missing_id =
            r#"{"extension":[{"code":"ext2","is_valid":false}],"value":"test_value_ext"}"#;
        let element_missing_id: Element<String, UnitTestExtension> =
            serde_json::from_str(json_missing_id).unwrap();
        assert_eq!(element_missing_id.id, None);
        assert_eq!(
            element_missing_id.extension,
            Some(vec![UnitTestExtension {
                code: "ext2".to_string(),
                is_valid: false
            }])
        );
        assert_eq!(element_missing_id.value, Some("test_value_ext".to_string()));

        // Object with only value
        let json_only_value_obj = r#"{"value":"test_value_only"}"#;
        let element_only_value: Element<String, UnitTestExtension> =
            serde_json::from_str(json_only_value_obj).unwrap();
        assert_eq!(element_only_value.id, None);
        assert_eq!(element_only_value.extension, None);
        assert_eq!(
            element_only_value.value,
            Some("test_value_only".to_string())
        );

        // Empty object
        let json_empty_obj = r#"{}"#;
        let element_empty_obj: Element<String, UnitTestExtension> =
            serde_json::from_str(json_empty_obj).unwrap();
        assert_eq!(element_empty_obj.id, None);
        assert_eq!(element_empty_obj.extension, None);
        assert_eq!(element_empty_obj.value, None); // Value is None when deserializing from empty object
    }

    #[test]
    fn test_deserialize_element_invalid_type() {
        // Array is not a valid representation for a single Element
        let json_array = r#"[1, 2, 3]"#;
        let result: Result<Element<i32, UnitTestExtension>, _> = serde_json::from_str(json_array);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid type: sequence, expected a primitive value (string, number, boolean), an object, or null"));

        // Boolean when expecting i32 (primitive case)
        let json_bool = r#"true"#;
        let result_bool: Result<Element<i32, UnitTestExtension>, _> =
            serde_json::from_str(json_bool);
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
        let result_obj_bool: Result<Element<i32, UnitTestExtension>, _> =
            serde_json::from_str(json_obj_bool_val);
        assert!(result_obj_bool.is_err());
        // Error comes from trying to deserialize the "value": true into Option<i32>
        assert!(result_obj_bool
            .unwrap_err()
            .to_string()
            .contains("invalid type: boolean `true`, expected i32"));

        // Define a simple struct that CANNOT deserialize from primitive types
        // Add Eq derive
        #[derive(Deserialize, Debug, PartialEq, Eq)]
        struct NonPrimitive {
            field: String,
        }

        // Try deserializing a primitive string into Element<NonPrimitive, _>
        let json_prim_str = r#""hello""#;
        let result_prim_nonprim: Result<Element<NonPrimitive, UnitTestExtension>, _> =
            serde_json::from_str(json_prim_str);
        assert!(result_prim_nonprim.is_err());
        // Error comes from V::deserialize failing inside the visitor
        assert!(result_prim_nonprim
            .unwrap_err()
            .to_string()
            .contains("invalid type: string \"hello\", expected struct NonPrimitive"));

        // Try deserializing an object into Element<NonPrimitive, _> (this should work if object has correct field)
        let json_obj_nonprim = r#"{"value": {"field": "world"}}"#;
        let result_obj_nonprim: Result<Element<NonPrimitive, UnitTestExtension>, _> =
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

    // Define a test struct that uses the FhirSerde derive
    // FhirSerde must be the only derive that generates Serialize/Deserialize impls
    #[derive(Debug, PartialEq, FhirSerde)] // Use FhirSerde derive
    struct FhirSerdeTestStruct {
        // Regular field
        name: Option<String>,

        // Field with potential extension (_birthDate) using type alias
        // FhirSerde should handle the 'birthDate'/'_birthDate' logic based on the field name.
        #[rustfmt::skip]
        birth_date: Option<r4::Date>, // Use type alias like in Patient

        // Another potentially extended field using type alias
        // FhirSerde should handle the 'isActive'/'_isActive' logic based on the field name.
        #[rustfmt::skip]
        is_active: Option<r4::Boolean>, // Use type alias

        // A non-element field for good measure
        count: Option<i32>,
    }

    #[test]
    fn test_fhir_serde_serialize() {
        // Case 1: Only primitive value for birthDate
        // Use r4::Date which is Element<String, Extension>
        let s1 = FhirSerdeTestStruct {
            name: Some("Test1".to_string()),
            birth_date: Some(r4::Date { // Construct using the alias type
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
        // Use r4::Date which is Element<String, Extension>
        let s2 = FhirSerdeTestStruct {
            name: Some("Test2".to_string()),
            birth_date: Some(r4::Date { // Construct using the alias type
                id: Some("bd-id".to_string()),
                extension: Some(vec![UnitTestExtension { // Use the test extension type
                    code: "note".to_string(),
                    is_valid: true,
                }]),
                value: None,
            }),
            is_active: None,
            count: None,
        };
        let json2 = serde_json::to_string(&s2).unwrap();
        // Expected output according to FHIR primitive extension pattern (_fieldName for extension object)
        let expected2 = r#"{"name":"Test2","_birthDate":{"id":"bd-id","extension":[{"code":"note","is_valid":true}]}}"#;
        assert_eq!(json2, expected2);

        // Case 3: Both primitive value and extension for birthDate
        // Use r4::Date and r4::Boolean
        let s3 = FhirSerdeTestStruct {
            name: Some("Test3".to_string()),
            birth_date: Some(r4::Date { // Construct using the alias type
                id: Some("bd-id-3".to_string()),
                extension: Some(vec![UnitTestExtension { // Use the test extension type
                    code: "text".to_string(),
                    is_valid: false,
                }]),
                value: Some("1970-03-30".to_string()),
            }),
            is_active: Some(r4::Boolean { // Construct using the alias type
                // Also test is_active field
                id: None,
                extension: None,
                value: Some(true),
            }),
            count: Some(3),
        };
        let json3 = serde_json::to_string(&s3).unwrap();
        // Expected output according to FHIR primitive extension pattern
        // "fieldName": value, "_fieldName": { id/extension }
        // "isActive": true (primitive only)
        let expected3 = r#"{"name":"Test3","birthDate":"1970-03-30","_birthDate":{"id":"bd-id-3","extension":[{"code":"text","is_valid":false}]},"isActive":true,"count":3}"#;
        assert_eq!(json3, expected3);

        // Case 4: birthDate field is None
        // Use r4::Boolean
        let s4 = FhirSerdeTestStruct {
            name: Some("Test4".to_string()),
            birth_date: None,
            is_active: Some(r4::Boolean { // Construct using the alias type
                // is_active has only extension
                id: None,
                extension: Some(vec![UnitTestExtension { // Use the test extension type
                    code: "flag".to_string(),
                    is_valid: true,
                }]),
                value: None,
            }),
            count: None,
        };
        let json4 = serde_json::to_string(&s4).unwrap();
        // Expected output according to FHIR primitive extension pattern (_fieldName for extension object)
        let expected4 =
            r#"{"name":"Test4","_isActive":{"extension":[{"code":"flag","is_valid":true}]}}"#;
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
        // Use r4::Date
        let expected1 = FhirSerdeTestStruct {
            name: Some("Test1".to_string()),
            birth_date: Some(r4::Date { // Construct using the alias type
                id: None,
                extension: None,
                value: Some("1970-03-30".to_string()),
            }),
            is_active: None,
            count: Some(1),
        };
        let s1: FhirSerdeTestStruct = serde_json::from_str(json1).unwrap();
        assert_eq!(s1, expected1); // Uncomment assertion

        // Case 2: Only extension for birthDate
        let json2 = r#"{"name":"Test2","_birthDate":{"id":"bd-id","extension":[{"code":"note","is_valid":true}]}}"#;
        // Use r4::Date
        let _expected2 = FhirSerdeTestStruct {
            // Prefixed unused variable
            name: Some("Test2".to_string()),
            birth_date: Some(r4::Date { // Construct using the alias type
                id: Some("bd-id".to_string()),
                extension: Some(vec![UnitTestExtension { // Use the test extension type
                    code: "note".to_string(),
                    is_valid: true,
                }]),
                value: None,
            }),
            is_active: None,
            count: None,
        };
        let _s2: FhirSerdeTestStruct = serde_json::from_str(json2).unwrap(); // Prefixed unused variable
        assert_eq!(_s2, _expected2); // Should now pass with macro fix

        // Case 3: Both primitive value and extension for birthDate and isActive
        let json3 = r#"{"name":"Test3","birthDate":"1970-03-30","_birthDate":{"id":"bd-id-3","extension":[{"code":"text","is_valid":false}]},"isActive":true,"_isActive":{"id":"active-id"},"count":3}"#;
        // Use r4::Date and r4::Boolean
        let _expected3 = FhirSerdeTestStruct {
            // Prefixed unused variable
            name: Some("Test3".to_string()),
            birth_date: Some(r4::Date { // Construct using the alias type
                id: Some("bd-id-3".to_string()),
                extension: Some(vec![UnitTestExtension { // Use the test extension type
                    code: "text".to_string(),
                    is_valid: false,
                }]),
                value: Some("1970-03-30".to_string()),
            }),
            is_active: Some(r4::Boolean { // Construct using the alias type
                id: Some("active-id".to_string()), // Merged from _isActive
                extension: None,                   // Merged from _isActive
                value: Some(true),                 // From isActive
            }),
            count: Some(3),
        };
        let _s3: FhirSerdeTestStruct = serde_json::from_str(json3).unwrap(); // Prefixed unused variable
        assert_eq!(_s3, _expected3); // Should now pass with macro fix

        // Case 4: birthDate field is missing, isActive has only extension
        let json4 =
            r#"{"name":"Test4","_isActive":{"extension":[{"code":"flag","is_valid":true}]}}"#;
        // Use r4::Boolean
        let _expected4 = FhirSerdeTestStruct {
            // Prefixed unused variable
            name: Some("Test4".to_string()),
            birth_date: None,
            is_active: Some(r4::Boolean { // Construct using the alias type
                id: None,
                extension: Some(vec![UnitTestExtension { // Use the test extension type
                    code: "flag".to_string(),
                    is_valid: true,
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
        // Use r4::Date
        let _expected6 = FhirSerdeTestStruct {
            // Prefixed unused variable
            name: None,
            birth_date: Some(r4::Date { // Construct using the alias type
                id: Some("bd-null".to_string()),
                extension: None,
                value: None, // Value is None because input was null
            }),
            is_active: None,
            count: None,
        };
        let _s6: FhirSerdeTestStruct = serde_json::from_str(json6).unwrap(); // Prefixed unused variable
        assert_eq!(_s6, _expected6); // Should now pass with macro fix

        // Case 7: Primitive value exists, but extension is null (should ignore null extension object)
        let json7 = r#"{"birthDate":"1999-09-09","_birthDate":null}"#;
        // Use r4::Date
        let _expected7 = FhirSerdeTestStruct {
            // Prefixed unused variable
            name: None,
            birth_date: Some(r4::Date { // Construct using the alias type
                id: None,
                extension: None,
                value: Some("1999-09-09".to_string()),
            }),
            is_active: None,
            count: None,
        };
        let _s7: FhirSerdeTestStruct = serde_json::from_str(json7).unwrap(); // Prefixed unused variable
        assert_eq!(_s7, _expected7); // Should now pass with macro fix

        // Case 8: Duplicate primitive field (should error)
        let json8 = r#"{"birthDate":"1970-03-30", "birthDate":"1971-04-01"}"#;
        let res8: Result<FhirSerdeTestStruct, _> = serde_json::from_str(json8);
        assert!(res8.is_err());
        assert!(res8
            .unwrap_err()
            .to_string()
            .contains("duplicate field `birthDate`"));

        // Case 9: Duplicate extension field (should error)
        let json9 = r#"{"_birthDate":{"id":"a"}, "_birthDate":{"id":"b"}}"#;
        let _res9: Result<FhirSerdeTestStruct, _> = serde_json::from_str(json9);
        // Prefixed unused variable
        assert!(_res9.is_err()); // Should now error due to duplicate field handled by macro visitor
        assert!(_res9
            .unwrap_err()
            .to_string()
            .contains("duplicate field `_birthDate`"));
    }
}
