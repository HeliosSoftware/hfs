//! JSON serialization and deserialization for FHIR resources.
//!
//! This module provides a familiar `serde_json`-like API for working with FHIR resources,
//! while internally using the context-driven serialization infrastructure to handle
//! FHIR-specific requirements like primitive extensions.
//!
//! # Examples
//!
//! ```rust,ignore
//! use helios_fhir::r4::Patient;
//! use helios_serde::json;
//!
//! // Create a patient
//! let patient = Patient::default();
//!
//! // Serialize to JSON string
//! let json = json::to_string(&patient)?;
//!
//! // Serialize to JSON bytes
//! let bytes = json::to_vec(&patient)?;
//!
//! // Deserialize from JSON string
//! let parsed: Patient = json::from_str(&json)?;
//!
//! // Deserialize from JSON bytes
//! let parsed: Patient = json::from_slice(&bytes)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::{DeserializationContext, Json, SerializationContext};
use serde::de::{DeserializeSeed, IntoDeserializer};
use serde::Serialize;

/// Serialize a FHIR resource to a JSON string.
///
/// # Arguments
///
/// * `value` - The FHIR resource to serialize
///
/// # Returns
///
/// A `Result` containing the JSON string or a serialization error.
///
/// # Examples
///
/// ```rust,ignore
/// use helios_fhir::r4::Patient;
/// use helios_serde::json;
///
/// let patient = Patient::default();
/// let json = json::to_string(&patient)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn to_string<T>(value: &T) -> Result<String, serde_json::Error>
where
    for<'a> SerializationContext<&'a T, Json>: Serialize,
{
    let ctx = SerializationContext::json(value);
    serde_json::to_string(&ctx)
}

/// Serialize a FHIR resource to a pretty-printed JSON string.
///
/// # Arguments
///
/// * `value` - The FHIR resource to serialize
///
/// # Returns
///
/// A `Result` containing the pretty-printed JSON string or a serialization error.
///
/// # Examples
///
/// ```rust,ignore
/// use helios_fhir::r4::Patient;
/// use helios_serde::json;
///
/// let patient = Patient::default();
/// let json = json::to_string_pretty(&patient)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn to_string_pretty<T>(value: &T) -> Result<String, serde_json::Error>
where
    for<'a> SerializationContext<&'a T, Json>: Serialize,
{
    let ctx = SerializationContext::json(value);
    serde_json::to_string_pretty(&ctx)
}

/// Serialize a FHIR resource to a JSON byte vector.
///
/// # Arguments
///
/// * `value` - The FHIR resource to serialize
///
/// # Returns
///
/// A `Result` containing the JSON bytes or a serialization error.
///
/// # Examples
///
/// ```rust,ignore
/// use helios_fhir::r4::Patient;
/// use helios_serde::json;
///
/// let patient = Patient::default();
/// let bytes = json::to_vec(&patient)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn to_vec<T>(value: &T) -> Result<Vec<u8>, serde_json::Error>
where
    for<'a> SerializationContext<&'a T, Json>: Serialize,
{
    let ctx = SerializationContext::json(value);
    serde_json::to_vec(&ctx)
}

/// Serialize a FHIR resource to a pretty-printed JSON byte vector.
///
/// # Arguments
///
/// * `value` - The FHIR resource to serialize
///
/// # Returns
///
/// A `Result` containing the pretty-printed JSON bytes or a serialization error.
pub fn to_vec_pretty<T>(value: &T) -> Result<Vec<u8>, serde_json::Error>
where
    for<'a> SerializationContext<&'a T, Json>: Serialize,
{
    let ctx = SerializationContext::json(value);
    serde_json::to_vec_pretty(&ctx)
}

/// Serialize a FHIR resource to a `serde_json::Value`.
pub fn to_value<T>(value: &T) -> Result<serde_json::Value, serde_json::Error>
where
    for<'a> SerializationContext<&'a T, Json>: Serialize,
{
    let ctx = SerializationContext::json(value);
    serde_json::to_value(&ctx)
}

/// Deserialize a FHIR resource from a JSON string.
///
/// # Type Parameters
///
/// * `T` - The FHIR resource type to deserialize into
///
/// # Arguments
///
/// * `s` - The JSON string to parse
///
/// # Returns
///
/// A `Result` containing the deserialized resource or a deserialization error.
///
/// # Examples
///
/// ```rust,ignore
/// use helios_fhir::r4::Patient;
/// use helios_serde::json;
///
/// let json = r#"{"resourceType":"Patient"}"#;
/// let patient: Patient = json::from_str(json)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn from_str<T>(s: &str) -> Result<T, serde_json::Error>
where
    for<'de> DeserializationContext<T, Json>: DeserializeSeed<'de, Value = T>,
{
    let ctx = DeserializationContext::<T, Json>::json();
    let mut deserializer = serde_json::Deserializer::from_str(s);
    ctx.deserialize(&mut deserializer)
}

/// Deserialize a FHIR resource from JSON bytes.
///
/// # Type Parameters
///
/// * `T` - The FHIR resource type to deserialize into
///
/// # Arguments
///
/// * `v` - The JSON bytes to parse
///
/// # Returns
///
/// A `Result` containing the deserialized resource or a deserialization error.
///
/// # Examples
///
/// ```rust,ignore
/// use helios_fhir::r4::Patient;
/// use helios_serde::json;
///
/// let json_bytes = br#"{"resourceType":"Patient"}"#;
/// let patient: Patient = json::from_slice(json_bytes)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn from_slice<T>(v: &[u8]) -> Result<T, serde_json::Error>
where
    for<'de> DeserializationContext<T, Json>: DeserializeSeed<'de, Value = T>,
{
    let ctx = DeserializationContext::<T, Json>::json();
    let mut deserializer = serde_json::Deserializer::from_slice(v);
    ctx.deserialize(&mut deserializer)
}

/// Deserialize a FHIR resource from a `serde_json::Value`.
pub fn from_value<T>(value: serde_json::Value) -> Result<T, serde_json::Error>
where
    for<'de> DeserializationContext<T, Json>: DeserializeSeed<'de, Value = T>,
{
    let ctx = DeserializationContext::<T, Json>::json();
    let deserializer = value.into_deserializer();
    ctx.deserialize(deserializer)
}

/// Deserialize a FHIR resource from an IO reader.
///
/// # Type Parameters
///
/// * `T` - The FHIR resource type to deserialize into
/// * `R` - The reader type (must implement `std::io::Read`)
///
/// # Arguments
///
/// * `rdr` - The reader containing JSON data
///
/// # Returns
///
/// A `Result` containing the deserialized resource or a deserialization error.
pub fn from_reader<R, T>(rdr: R) -> Result<T, serde_json::Error>
where
    R: std::io::Read,
    for<'de> DeserializationContext<T, Json>: DeserializeSeed<'de, Value = T>,
{
    let ctx = DeserializationContext::<T, Json>::json();
    let mut deserializer = serde_json::Deserializer::from_reader(rdr);
    ctx.deserialize(&mut deserializer)
}
