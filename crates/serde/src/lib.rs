//! # Helios FHIR Server Serialization Module
//!
//! This crate provides version-agnostic JSON and XML serialization for FHIR resources.
//!
//! ## Features
//!
//! - **JSON Support**: Thin wrappers around `serde_json` that leverage the existing
//!   `FhirSerde` derive macro for correct FHIR JSON representation. Always available.
//! - **XML Support**: Custom `serde::Serializer` and `serde::Deserializer` implementations
//!   that stream directly to/from FHIR XML format without materializing JSON intermediates.
//!   Requires the `xml` feature flag: `helios-serde = { features = ["xml"] }`.
//! - **Version Agnostic**: Works with all FHIR versions (R4, R4B, R5, R6) through the
//!   `Element<V, E>` infrastructure.
//!
//! ## Architecture
//!
//! The crate uses a streaming approach for maximum performance:
//!
//! - **JSON Layer**: Direct delegation to `serde_json` functions
//! - **XML Layer**: Custom `Serializer`/`Deserializer` trait implementations that:
//!   - Receive serialize/deserialize calls as the resource is traversed
//!   - Buffer minimally (only what's needed for FHIR pattern detection)
//!   - Write/read quick-xml events directly
//!
//! ## FHIR JSON ↔ XML Mapping
//!
//! The XML implementation handles FHIR's unique serialization patterns:
//!
//! | JSON Pattern | XML Pattern |
//! |--------------|-------------|
//! | `{"active": true}` | `<active value="true"/>` |
//! | `{"birthDate": "1974-12-25", "_birthDate": {"id": "123"}}` | `<birthDate id="123" value="1974-12-25"/>` |
//! | `{"given": ["John", "Doe"]}` | `<given value="John"/><given value="Doe"/>` |
//! | `{"given": ["A", null], "_given": [null, {"id": "123"}]}` | `<given value="A"/><given id="123"/>` |
//!
//! ## Examples
//!
//! ### JSON Serialization
//!
//! ```ignore
//! use helios_serde::json::{to_json_string, from_json_str};
//! use helios_fhir::r4::Patient;
//!
//! // Serialize to JSON
//! let patient = Patient::default();
//! let json = to_json_string(&patient)?;
//!
//! // Deserialize from JSON
//! let patient: Patient = from_json_str(&json)?;
//! ```
//!
//! ### XML Serialization (Coming Soon)
//!
//! ```ignore
//! use helios_serde::xml::{to_xml_string, from_xml_str};
//! use helios_fhir::r4::Patient;
//!
//! // Serialize to XML
//! let patient = Patient::default();
//! let xml = to_xml_string(&patient)?;
//!
//! // Deserialize from XML
//! let patient: Patient = from_xml_str(&xml)?;
//! ```

// Not referenced by the library code, but linked on purpose: the crate's
// `R4`/`R4B`/`R5`/`R6`/`xml` features forward to `helios-fhir`, and the
// tests and examples use its resource types.
use helios_fhir as _;

pub mod error;
pub mod json;
#[cfg(feature = "xml")]
pub mod xml;

// Re-export common types and functions
pub use error::{Result, SerdeError};

// Re-export JSON functions at top level for convenience
pub use json::{
    from_json_slice, from_json_str, from_json_value, to_json_string, to_json_string_pretty,
    to_json_value, to_json_vec,
};

// Re-export XML functions at top level for convenience
#[cfg(feature = "xml")]
pub use xml::{
    from_xml_reader, from_xml_slice, from_xml_str, to_xml_string, to_xml_vec, to_xml_writer,
};
