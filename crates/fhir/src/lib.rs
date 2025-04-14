use rust_decimal::Decimal;
use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer, MapAccess, Visitor},
    ser::{SerializeStruct, Serializer},
};
// Removed unused RawValue import
use std::marker::PhantomData;

// Store the original string representation and optionally the parsed Decimal value.
#[derive(Debug, Clone)]
pub struct PreciseDecimal {
    // Store None if the original string couldn't be parsed into Decimal (e.g., out of range)
    value: Option<Decimal>,
    original_string: String,
}

// Implement comparison based on the numerical value (Option<Decimal>)
impl PartialEq for PreciseDecimal {
    fn eq(&self, other: &Self) -> bool {
        // Compare the Option<Decimal> values. None == None is true.
        self.value == other.value
    }
}
impl Eq for PreciseDecimal {}

impl PartialOrd for PreciseDecimal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        // Compare the Option<Decimal> values. None is considered less than Some(_).
        self.value.partial_cmp(&other.value)
    }
}

impl Ord for PreciseDecimal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Compare the Option<Decimal> values. None is considered less than Some(_).
        self.value.cmp(&other.value)
    }
}

// Provide methods to access the inner value if needed
impl PreciseDecimal {
    /// Creates a new PreciseDecimal from its parts.
    /// Use this when you have both the parsed value (or know it failed) and the original string.
    pub fn from_parts(value: Option<Decimal>, original_string: String) -> Self {
        Self {
            value,
            original_string,
        }
    }

    /// Returns the parsed Decimal value, if parsing was successful.
    pub fn value(&self) -> Option<Decimal> {
        self.value
    }

    /// Returns the original string representation.
    pub fn original_string(&self) -> &str {
        &self.original_string
    }
}

// Implement From<Decimal> to allow easy conversion, deriving the string representation.
impl From<Decimal> for PreciseDecimal {
    fn from(value: Decimal) -> Self {
        // Convert the Decimal to string to store as original_string.
        let original_string = value.to_string();
        // When creating from a valid Decimal, the value is Some(value).
        Self {
            value: Some(value),
            original_string,
        }
    }
}

impl Serialize for PreciseDecimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Use RawValue to serialize the original string directly as a JSON number.
        // This ensures the exact string format (including trailing zeros) is preserved.
        match serde_json::value::RawValue::from_string(self.original_string.clone()) {
            Ok(raw_value) => raw_value.serialize(serializer),
            Err(e) => Err(serde::ser::Error::custom(format!(
                "Failed to create RawValue from PreciseDecimal original_string '{}': {}",
                self.original_string, e
            ))),
        }
    }
}

// --- Visitor for PreciseDecimal Deserialization ---
struct PreciseDecimalVisitor;

impl<'de> Visitor<'de> for PreciseDecimalVisitor {
    type Value = PreciseDecimal;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a number or string that can represent a decimal")
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // Use v.to_string() as the original string representation from f64
        // Note: This might differ from the exact JSON source (e.g., "1E-22" vs "1e-22")
        // but is the best we can do when receiving an f64.
        self.parse_and_construct(v.to_string())
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.parse_and_construct(v.to_string())
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.parse_and_construct(v.to_string())
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // Use the provided string slice directly
        self.parse_and_construct(v.to_string())
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // Use the provided string directly
        self.parse_and_construct(v)
    }

     fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
        where
            E: de::Error,
    {
        // Use the provided borrowed string slice directly
        self.parse_and_construct(v.to_string())
    }

    // Handle cases where PreciseDecimal might be nested inside an object like {"value": 123.45}
    // This requires deserializing the inner value first to get its string representation.
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
    {
        let mut value_str: Option<String> = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "value" {
                if value_str.is_some() {
                    return Err(de::Error::duplicate_field("value"));
                }
                // Deserialize the value field flexibly (could be number or string)
                // We need the string representation. Deserialize as Value first.
                let temp_value: serde_json::Value = map.next_value()?;
                match temp_value {
                    serde_json::Value::Number(n) => value_str = Some(n.to_string()),
                    serde_json::Value::String(s) => value_str = Some(s),
                     serde_json::Value::Null => {
                        // If value is explicitly null in the object, treat as error.
                         return Err(de::Error::invalid_value(de::Unexpected::Unit, &"a number or string for decimal value"));
                    }
                    _ => return Err(de::Error::invalid_type(de::Unexpected::Other("unexpected type in value field"), &"a number or string"))
                }
            } else {
                // Ignore other fields
                let _ = map.next_value::<de::IgnoredAny>()?;
            }
        }

        match value_str {
            Some(s) => self.parse_and_construct(s),
            None => Err(de::Error::missing_field("value")),
        }
    }
}

// Implement helper methods directly on the visitor struct
impl PreciseDecimalVisitor {
    // Helper to perform the parse and construction, taking the original string
    fn parse_and_construct<E>(&self, s: String) -> Result<PreciseDecimal, E>
    where
        E: de::Error,
    {
        let parsed_value = s.parse::<Decimal>().ok();
        Ok(PreciseDecimal::from_parts(parsed_value, s))
    }
}

// Deserialize implementation for PreciseDecimal using the visitor
impl<'de> Deserialize<'de> for PreciseDecimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use the visitor to handle different primitive types directly
        deserializer.deserialize_any(PreciseDecimalVisitor)
    }
}

// --- End PreciseDecimal ---

// --- Visitor for DecimalElement Object Deserialization ---
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
        // Use PreciseDecimal::deserialize for the value field
        let mut value: Option<PreciseDecimal> = None;

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
                    // Deserialize directly into Option<PreciseDecimal> using its impl
                    // Important: Use map.next_value_seed(option_seed) if value itself could be null in JSON
                    // Assuming "value": null means the field is absent or PreciseDecimal should be None
                    // Let PreciseDecimal::deserialize handle null if needed within its logic
                    value = Some(map.next_value()?);
                }
                _ => { let _ = map.next_value::<de::IgnoredAny>()?; }
            }
        }

        Ok(DecimalElement { id, extension, value })
    }
}


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

// Note: Cannot add Copy if V or E are not Copy (like String, Vec)
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct Element<V, E> {
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
            // Restore value serialization for direct Element serialization
            if let Some(value) = &self.value {
                state.serialize_field("value", value)?;
            }
            state.end()
        }
    }
}

// Add Clone and Default derives
#[derive(Debug, PartialEq, Eq, Clone, Default)]
// Remove serde attributes as they are not used without derive
pub struct DecimalElement<E> {
    pub id: Option<String>,
    pub extension: Option<Vec<E>>,
    // Use the PreciseDecimal wrapper for the value field
    pub value: Option<PreciseDecimal>,
}

impl<E> DecimalElement<E> {
    /// Creates a new DecimalElement with the given value, setting id and extension to None.
    /// The original string representation is derived automatically from the Decimal value.
    ///
    /// # Example
    /// ```
    /// # use fhir::r4::Decimal; // Assuming Decimal is DecimalElement<Extension>
    /// # use rust_decimal_macros::dec;
    /// let decimal_value = dec!(123.45);
    /// let fhir_decimal = Decimal::new(decimal_value);
    /// assert_eq!(fhir_decimal.value.as_ref().map(|pd| pd.value()), Some(decimal_value));
    /// assert_eq!(fhir_decimal.value.map(|pd| pd.original_string().to_string()), Some("123.45".to_string()));
    /// assert!(fhir_decimal.id.is_none());
    /// assert!(fhir_decimal.extension.is_none());
    /// ```
    pub fn new(value: Decimal) -> Self {
        // Use the From<Decimal> impl for PreciseDecimal to create it,
        // which automatically handles storing the original string representation.
        let precise_value = PreciseDecimal::from(value);
        Self {
            id: None,
            extension: None, // Assuming E is typically Vec<Extension> or similar, default should be None
            value: Some(precise_value),
        }
    }
}

// Custom Deserialize for DecimalElement<E> using the visitor pattern
impl<'de, E> Deserialize<'de> for DecimalElement<E>
where
    E: Deserialize<'de> + Default, // Keep Default bound for visit_none/visit_unit
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AnyDecimalElementVisitor<E>(PhantomData<E>);

        impl<'de, E> Visitor<'de> for AnyDecimalElementVisitor<E>
        where
            E: Deserialize<'de> + Default,
        {
            type Value = DecimalElement<E>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a decimal number, string, object, or null")
            }

            // Handle primitive types by deserializing PreciseDecimal and wrapping it
            fn visit_bool<Er>(self, v: bool) -> Result<Self::Value, Er> where Er: de::Error {
                // Decimal cannot come from bool
                Err(de::Error::invalid_type(de::Unexpected::Bool(v), &self))
            }
            fn visit_i64<Er>(self, v: i64) -> Result<Self::Value, Er> where Er: de::Error {
                // Delegate to PreciseDecimal::deserialize which handles intermediate Value
                PreciseDecimal::deserialize(de::value::I64Deserializer::new(v))
                    .map(|pd| DecimalElement { id: None, extension: None, value: Some(pd) })
            }
            fn visit_u64<Er>(self, v: u64) -> Result<Self::Value, Er> where Er: de::Error {
                 // Delegate to PreciseDecimal::deserialize which handles intermediate Value
                PreciseDecimal::deserialize(de::value::U64Deserializer::new(v))
                    .map(|pd| DecimalElement { id: None, extension: None, value: Some(pd) })
            }
            fn visit_f64<Er>(self, v: f64) -> Result<Self::Value, Er> where Er: de::Error {
                 // Delegate to PreciseDecimal::deserialize which handles intermediate Value
                PreciseDecimal::deserialize(de::value::F64Deserializer::new(v))
                    .map(|pd| DecimalElement { id: None, extension: None, value: Some(pd) })
            }
            fn visit_str<Er>(self, v: &str) -> Result<Self::Value, Er> where Er: de::Error {
                 // Delegate to PreciseDecimal::deserialize which handles intermediate Value
                PreciseDecimal::deserialize(de::value::StrDeserializer::new(v))
                    .map(|pd| DecimalElement { id: None, extension: None, value: Some(pd) })
            }
            fn visit_string<Er>(self, v: String) -> Result<Self::Value, Er> where Er: de::Error {
                 // Delegate to PreciseDecimal::deserialize which handles intermediate Value
                PreciseDecimal::deserialize(de::value::StringDeserializer::new(v))
                    .map(|pd| DecimalElement { id: None, extension: None, value: Some(pd) })
            }
             fn visit_borrowed_str<Er>(self, v: &'de str) -> Result<Self::Value, Er> where Er: de::Error {
                 // Delegate to PreciseDecimal::deserialize which handles intermediate Value
                PreciseDecimal::deserialize(de::value::BorrowedStrDeserializer::new(v))
                    .map(|pd| DecimalElement { id: None, extension: None, value: Some(pd) })
            }
            // Decimal cannot come from bytes
            fn visit_bytes<Er>(self, v: &[u8]) -> Result<Self::Value, Er> where Er: de::Error {
                 Err(de::Error::invalid_type(de::Unexpected::Bytes(v), &self))
            }
            fn visit_byte_buf<Er>(self, v: Vec<u8>) -> Result<Self::Value, Er> where Er: de::Error {
                 Err(de::Error::invalid_type(de::Unexpected::Bytes(&v), &self))
            }

            // Handle null
            fn visit_none<Er>(self) -> Result<Self::Value, Er> where Er: de::Error {
                Ok(DecimalElement::default()) // Default has value: None
            }
            fn visit_unit<Er>(self) -> Result<Self::Value, Er> where Er: de::Error {
                Ok(DecimalElement::default())
            }

            // Handle Option<T> by visiting Some
            fn visit_some<De>(self, deserializer: De) -> Result<Self::Value, De::Error>
            where
                De: Deserializer<'de>,
            {
                deserializer.deserialize_any(self)
            }

            // Handle object
            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                // Deserialize the map using DecimalElementObjectVisitor
                let map_deserializer = de::value::MapAccessDeserializer::new(map);
                map_deserializer.deserialize_map(DecimalElementObjectVisitor(PhantomData))
            }

            // We don't expect sequences for a single DecimalElement
            fn visit_seq<A>(self, _seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                Err(de::Error::invalid_type(de::Unexpected::Seq, &self))
            }
        }

        deserializer.deserialize_any(AnyDecimalElementVisitor(PhantomData))
    }
}

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
                // If value is also None, serialize as null
                // based on updated test_serialize_decimal_with_no_fields
                return serializer.serialize_none();
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
