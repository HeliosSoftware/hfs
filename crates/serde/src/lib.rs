//! # helios_serde - Serialization Support for FHIR Resources
//!
//! This crate provides format-aware serialization and deserialization for FHIR resources.
//! It acts as a bridge between the generated FHIR types and various serialization formats.
//!
//! ## Architecture
//!
//! The crate uses a context-driven approach where serialization and deserialization are
//! performed through format-specific contexts. This design enables:
//! - Zero-overhead format specialization via compile-time type parameters
//! - Clean separation between FHIR semantics and format encoding
//! - Easy extension to support new formats (XML, CSV, etc.)
//!
//! ## Usage
//!
//! ```rust,ignore
//! use helios_fhir::r4::Patient;
//! use helios_serde::json;
//!
//! // Serialize a patient to JSON
//! let patient = Patient::default();
//! let json_string = json::to_string(&patient)?;
//!
//! // Deserialize from JSON
//! let patient: Patient = json::from_str(&json_string)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Format Modules
//!
//! - [`json`] - JSON serialization/deserialization with FHIR primitive extension support

use serde::de::{DeserializeSeed, Deserializer};
use serde::ser::{Serialize, Serializer};
use std::convert::TryFrom;
use std::fmt;
use std::marker::PhantomData;

pub mod json;

// =============================================================================
// Context-Driven Serialization Infrastructure
// =============================================================================

/// Format marker type for JSON serialization/deserialization.
///
/// This zero-sized type is used as a compile-time marker to enable
/// format-specific behavior without runtime overhead.
#[derive(Debug, Clone, Copy)]
pub struct Json;

/// Context wrapper for serialization that enables format-aware behavior.
///
/// `SerializationContext` wraps a value and a format marker type, allowing
/// the generated `Serialize` implementations to be specialized per format
/// via trait implementations.
///
/// # Type Parameters
///
/// * `V` - The value type being serialized
/// * `F` - The format marker type (e.g., `Json`)
pub struct SerializationContext<V, F> {
    /// The value being serialized
    pub data: V,
    /// Format marker (zero-sized, compile-time only)
    _format: PhantomData<F>,
}

impl<V, F> SerializationContext<V, F> {
    /// Create a new serialization context with the given value and format.
    pub fn new(data: V) -> Self {
        Self {
            data,
            _format: PhantomData,
        }
    }
}

impl<V> SerializationContext<V, Json> {
    /// Create a JSON serialization context for the given value.
    pub fn json(data: V) -> Self {
        Self::new(data)
    }
}

/// Context wrapper for deserialization that enables format-aware behavior.
///
/// `DeserializationContext` serves as a `DeserializeSeed` implementation,
/// allowing format-specific deserialization logic to be injected at compile time.
///
/// # Type Parameters
///
/// * `V` - The value type being deserialized
/// * `F` - The format marker type (e.g., `Json`)
pub struct DeserializationContext<V, F> {
    /// Value type marker (zero-sized, compile-time only)
    _value: PhantomData<V>,
    /// Format marker (zero-sized, compile-time only)
    _format: PhantomData<F>,
}

impl<V, F> DeserializationContext<V, F> {
    /// Create a new deserialization context for the given type and format.
    pub fn new() -> Self {
        Self {
            _value: PhantomData,
            _format: PhantomData,
        }
    }
}

impl<V, F> Default for DeserializationContext<V, F> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> DeserializationContext<V, Json> {
    /// Create a JSON deserialization context for the given type.
    pub fn json() -> Self {
        Self::new()
    }
}

// =============================================================================
// Bridging Traits (for orphan rule compliance)
// =============================================================================

/// Internal trait for types that can be serialized in a FHIR-aware context.
///
/// # Why This Trait Exists (Orphan Rule Compliance)
///
/// This trait solves a fundamental Rust orphan rule problem. Without it, we would need to
/// implement `Serialize` (from serde) for `SerializationContext<&Patient, Json>`, but:
///
/// - `Serialize` is foreign (defined in serde)
/// - `SerializationContext` is foreign (defined here in helios-serde) from helios-fhir's perspective
/// - Even though `Patient` is local to helios-fhir, it's "covered" by the wrapper types
///
/// Rust's orphan rules prevent this because neither the trait nor the outermost type is local.
///
/// ## The Bridging Solution
///
/// This trait breaks the problem into two legal steps:
///
/// 1. **helios-fhir implements FhirSerialize for Patient**: Legal because FhirSerialize is a dependency trait
/// 2. **helios-serde implements Serialize for SerializationContext<&T, F> where T: FhirSerialize<F>**:
///    Legal because SerializationContext is local to helios-serde
///
/// The blanket impl effectively says: "Any type T that implements FhirSerialize automatically
/// gets Serialize when wrapped in SerializationContext".
///
/// # Implementation
///
/// This trait is automatically implemented by the `FhirSerde` derive macro. Users should
/// never implement this trait manually.
pub trait FhirSerialize<F> {
    fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer;
}

/// Internal trait for types that can be deserialized in a FHIR-aware context.
///
/// # Why This Trait Exists (Orphan Rule Compliance)
///
/// Similar to `FhirSerialize`, this trait solves an orphan rule problem for deserialization.
/// Without it, we would need to implement `DeserializeSeed` (from serde) for
/// `DeserializationContext<Patient, Json>`, which violates the orphan rules for the same
/// reasons described in `FhirSerialize`.
///
/// This trait serves as a bridge that allows:
///
/// 1. **helios-fhir to implement FhirDeserialize for Patient**: Legal because FhirDeserialize is a dependency trait
/// 2. **helios-serde to provide blanket DeserializeSeed impl**: Legal because DeserializationContext is local
///
/// # Implementation
///
/// This trait is automatically implemented by the `FhirSerde` derive macro. Users should
/// never implement this trait manually.
pub trait FhirDeserialize<F>: Sized {
    fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>;
}

// =============================================================================
// Blanket Implementations (bridge from FhirSerialize/Deserialize to Serialize/DeserializeSeed)
// =============================================================================

// Blanket impl: if T implements FhirSerialize<F>, then SerializationContext<&T, F> implements Serialize
impl<T, F> Serialize for SerializationContext<&T, F>
where
    T: FhirSerialize<F>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.data.fhir_serialize(serializer)
    }
}

// Blanket impl: if T implements FhirDeserialize<F>, then DeserializationContext<T, F> implements DeserializeSeed
impl<'de, T, F> DeserializeSeed<'de> for DeserializationContext<T, F>
where
    T: FhirDeserialize<F>,
{
    type Value = T;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::fhir_deserialize(deserializer)
    }
}

// =============================================================================
// Generic Implementations for Standard Types
// =============================================================================

// FhirSerialize/Deserialize implementations for primitive types
// These types already have Serialize/Deserialize, so we just delegate

macro_rules! impl_fhir_serde_for_primitive {
    ($($t:ty),*) => {
        $(
            impl FhirSerialize<Json> for $t {
                fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: Serializer,
                {
                    Serialize::serialize(self, serializer)
                }
            }

            impl FhirDeserialize<Json> for $t {
                fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
                where
                    D: Deserializer<'de>,
                {
                    <$t as serde::Deserialize>::deserialize(deserializer)
                }
            }
        )*
    };
}

// Implement for common primitive and standard library types
impl_fhir_serde_for_primitive!(String, bool, char);

macro_rules! impl_fhir_float {
    ($($t:ty),* $(,)?) => {
        $(
            impl FhirSerialize<Json> for $t {
                fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: Serializer,
                {
                    Serialize::serialize(self, serializer)
                }
            }

            impl FhirDeserialize<Json> for $t {
                fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
                where
                    D: Deserializer<'de>,
                {
                    struct FloatVisitor;

                    impl FloatVisitor {
                        fn parse_from_str<E>(value: &str) -> Result<$t, E>
                        where
                            E: serde::de::Error,
                        {
                            let trimmed = value.trim();
                            if trimmed.is_empty() {
                                return Err(E::invalid_value(
                                    serde::de::Unexpected::Str(value),
                                    &concat!(
                                        "a valid ",
                                        stringify!($t),
                                        " or stringified floating point number"
                                    ),
                                ));
                            }

                            trimmed.parse::<$t>().map_err(|_| {
                                E::invalid_value(
                                    serde::de::Unexpected::Str(value),
                                    &concat!(
                                        "a valid ",
                                        stringify!($t),
                                        " or stringified floating point number"
                                    ),
                                )
                            })
                        }
                    }

                    impl<'de> serde::de::Visitor<'de> for FloatVisitor {
                        type Value = $t;

                        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                            formatter.write_str(concat!(
                                "a floating point number or string containing a ",
                                stringify!($t)
                            ))
                        }

                        fn visit_f32<E>(self, v: f32) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Ok(v as Self::Value)
                        }

                        fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Ok(v as Self::Value)
                        }

                        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Ok(v as Self::Value)
                        }

                        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Ok(v as Self::Value)
                        }

                        fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Ok(v as Self::Value)
                        }

                        fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Ok(v as Self::Value)
                        }

                        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Self::parse_from_str(v)
                        }

                        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            self.visit_str(&v)
                        }

                        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                        where
                            A: serde::de::MapAccess<'de>,
                        {
                            use serde::de::Error;

                            while let Some(key) = map.next_key::<String>()? {
                                if key == crate::json::SERDE_NUMBER_SENTINEL {
                                    let serialized_number: String = map.next_value()?;
                                    return Self::parse_from_str::<A::Error>(&serialized_number);
                                } else {
                                    let _ = map.next_value::<serde::de::IgnoredAny>()?;
                                }
                            }

                            Err(Error::invalid_type(
                                serde::de::Unexpected::Map,
                                &concat!(
                                    "a valid ",
                                    stringify!($t),
                                    ", stringified float, or serde_json arbitrary precision map"
                                ),
                            ))
                        }
                    }

                    deserializer.deserialize_any(FloatVisitor)
                }
            }
        )*
    };
}

impl_fhir_float!(f32, f64);

macro_rules! impl_fhir_integer {
    ($($t:ty),* $(,)?) => {
        $(
            impl FhirSerialize<Json> for $t {
                fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: Serializer,
                {
                    Serialize::serialize(self, serializer)
                }
            }

            impl FhirDeserialize<Json> for $t {
                fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
                where
                    D: Deserializer<'de>,
                {
                    struct IntegerVisitor;

                    impl<'de> serde::de::Visitor<'de> for IntegerVisitor {
                        type Value = $t;

                        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                            formatter.write_str(concat!(
                                "an integer or string containing an integer representable as ",
                                stringify!($t)
                            ))
                        }

                        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Self::Value::try_from(v).map_err(|err| {
                                E::custom(format!(
                                    "value {} is out of range for {}: {}",
                                    v,
                                    stringify!($t),
                                    err
                                ))
                            })
                        }

                        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Self::Value::try_from(v).map_err(|err| {
                                E::custom(format!(
                                    "value {} is out of range for {}: {}",
                                    v,
                                    stringify!($t),
                                    err
                                ))
                            })
                        }

                        fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Self::Value::try_from(v).map_err(|err| {
                                E::custom(format!(
                                    "value {} is out of range for {}: {}",
                                    v,
                                    stringify!($t),
                                    err
                                ))
                            })
                        }

                        fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            Self::Value::try_from(v).map_err(|err| {
                                E::custom(format!(
                                    "value {} is out of range for {}: {}",
                                    v,
                                    stringify!($t),
                                    err
                                ))
                            })
                        }

                        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            let trimmed = v.trim();
                            if trimmed.is_empty() {
                                return Err(E::invalid_value(
                                    serde::de::Unexpected::Str(v),
                                    &concat!(
                                        "a valid ",
                                        stringify!($t),
                                        " or stringified integer"
                                    ),
                                ));
                            }

                            trimmed.parse::<Self::Value>().map_err(|_| {
                                E::invalid_value(
                                    serde::de::Unexpected::Str(v),
                                    &concat!(
                                        "a valid ",
                                        stringify!($t),
                                        " or stringified integer"
                                    ),
                                )
                            })
                        }

                        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
                        where
                            E: serde::de::Error,
                        {
                            self.visit_str(&v)
                        }
                    }

                    deserializer.deserialize_any(IntegerVisitor)
                }
            }
        )*
    };
}

impl_fhir_integer!(i8, i16, i32, i64, i128, u8, u16, u32, u64, u128);

// FhirSerialize/Deserialize for serde_json::Value (used in element primitives)
impl FhirSerialize<Json> for serde_json::Value {
    fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Serialize::serialize(self, serializer)
    }
}

impl FhirDeserialize<Json> for serde_json::Value {
    fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        <serde_json::Value as serde::Deserialize>::deserialize(deserializer)
    }
}

// FhirSerialize impl for Option<T>
impl<T> FhirSerialize<Json> for Option<T>
where
    T: FhirSerialize<Json>,
{
    fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Some(value) => value.fhir_serialize(serializer),
            None => serializer.serialize_none(),
        }
    }
}

// FhirSerialize impl for Vec<T>
impl<T> FhirSerialize<Json> for Vec<T>
where
    T: FhirSerialize<Json>,
{
    fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for item in self {
            let ctx = SerializationContext::json(item);
            seq.serialize_element(&ctx)?;
        }
        seq.end()
    }
}

// FhirSerialize impl for Box<T>
impl<T> FhirSerialize<Json> for Box<T>
where
    T: FhirSerialize<Json>,
{
    fn fhir_serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        (**self).fhir_serialize(serializer)
    }
}

// FhirDeserialize impl for Option<T>
impl<T> FhirDeserialize<Json> for Option<T>
where
    T: FhirDeserialize<Json>,
{
    fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de;

        struct OptionVisitor<T> {
            _marker: PhantomData<T>,
        }

        impl<'de, T> de::Visitor<'de> for OptionVisitor<T>
        where
            T: FhirDeserialize<Json>,
        {
            type Value = Option<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("an optional value")
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
                T::fhir_deserialize(deserializer).map(Some)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(None)
            }
        }

        deserializer.deserialize_option(OptionVisitor {
            _marker: PhantomData,
        })
    }
}

// FhirDeserialize impl for Vec<T>
impl<T> FhirDeserialize<Json> for Vec<T>
where
    T: FhirDeserialize<Json>,
{
    fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de;

        struct VecVisitor<T> {
            _marker: PhantomData<T>,
        }

        impl<'de, T> de::Visitor<'de> for VecVisitor<T>
        where
            T: FhirDeserialize<Json>,
        {
            type Value = Vec<T>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a sequence")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let mut vec = Vec::new();
                while let Some(elem) =
                    seq.next_element_seed(DeserializationContext::<T, Json>::json())?
                {
                    vec.push(elem);
                }
                Ok(vec)
            }
        }

        deserializer.deserialize_seq(VecVisitor {
            _marker: PhantomData,
        })
    }
}

// FhirDeserialize impl for Box<T>
impl<T> FhirDeserialize<Json> for Box<T>
where
    T: FhirDeserialize<Json>,
{
    fn fhir_deserialize<'de, D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        T::fhir_deserialize(deserializer).map(Box::new)
    }
}
