//! # FHIR Model Infrastructure
//!
//! This module provides the foundational types and infrastructure that support the
//! generated FHIR specification implementations. It contains hand-coded types that
//! enable the generated code to handle FHIR's complex requirements for precision,
//! extensions, and cross-version compatibility.
//!
//! ## Architecture
//!
//! The FHIR crate is organized as follows:
//! - **Generated modules** (`r4.rs`, `r4b.rs`, `r5.rs`, `r6.rs`): Complete FHIR type implementations
//! - **Infrastructure module** (`lib.rs`): Foundational types used by generated code
//! - **Test modules**: Validation against official FHIR examples
//!
//! ## Key Infrastructure Types
//!
//! - [`PreciseDecimal`] - High-precision decimal arithmetic preserving original string format
//! - [`Element<T, Extension>`] - Base container for FHIR elements with extension support
//! - [`DecimalElement<Extension>`] - Specialized element for decimal values
//! - [`FhirVersion`] - Version enumeration for multi-version support
//!
//! ## Usage Example
//!
//! ```rust
//! use fhir::r5::{Patient, HumanName};
//! use fhir::PreciseDecimal;
//! use rust_decimal::Decimal;
//!
//! // Create a patient with precise decimal handling
//! let patient = Patient {
//!     name: Some(vec![HumanName {
//!         family: Some("Doe".to_string()),
//!         given: Some(vec!["John".to_string()]),
//!         ..Default::default()
//!     }]),
//!     ..Default::default()
//! };
//!
//! // Work with precise decimals
//! let precise = PreciseDecimal::from(Decimal::new(12340, 3)); // 12.340
//! ```

use fhirpath_support::{EvaluationResult, IntoEvaluationResult};
use rust_decimal::Decimal;
use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer, MapAccess, Visitor},
    ser::{SerializeStruct, Serializer},
};
use std::marker::PhantomData;

/// High-precision decimal type that preserves original string representation.
///
/// FHIR requires that decimal values maintain their original precision and format
/// when serialized back to JSON. This type stores both the parsed `Decimal` value
/// for mathematical operations and the original string for serialization.
///
/// # FHIR Precision Requirements
///
/// FHIR decimal values must:
/// - Preserve trailing zeros (e.g., "12.340" vs "12.34")
/// - Maintain original precision during round-trip serialization
/// - Support high-precision arithmetic without floating-point errors
/// - Handle edge cases like very large or very small numbers
///
/// # Examples
///
/// ```rust
/// use fhir::PreciseDecimal;
/// use rust_decimal::Decimal;
///
/// // Create from Decimal (derives string representation)
/// let precise = PreciseDecimal::from(Decimal::new(12340, 3)); // 12.340
/// assert_eq!(precise.original_string(), "12.340");
///
/// // Create with specific string format
/// let precise = PreciseDecimal::from_parts(
///     Some(Decimal::new(1000, 2)), 
///     "10.00".to_string()
/// );
/// assert_eq!(precise.original_string(), "10.00");
/// ```
#[derive(Debug, Clone)]
pub struct PreciseDecimal {
    /// The parsed decimal value, `None` if parsing failed (e.g., out of range)
    value: Option<Decimal>,
    /// The original string representation preserving format and precision
    original_string: String,
}

/// Implements equality comparison based on the parsed decimal value.
///
/// Two `PreciseDecimal` values are equal if their parsed `Decimal` values are equal,
/// regardless of their original string representations. This enables mathematical
/// equality while preserving string format for serialization.
///
/// # Examples
///
/// ```rust
/// use fhir::PreciseDecimal;
/// use rust_decimal::Decimal;
///
/// let a = PreciseDecimal::from_parts(Some(Decimal::new(100, 1)), "10.0".to_string());
/// let b = PreciseDecimal::from_parts(Some(Decimal::new(1000, 2)), "10.00".to_string());
/// assert_eq!(a, b); // Same decimal value (10.0 == 10.00)
/// ```
impl PartialEq for PreciseDecimal {
    fn eq(&self, other: &Self) -> bool {
        // Compare parsed decimal values for mathematical equality
        self.value == other.value
    }
}

/// Marker trait implementation indicating total equality for `PreciseDecimal`.
impl Eq for PreciseDecimal {}

/// Implements partial ordering based on the parsed decimal value.
///
/// Ordering is based on the mathematical value of the decimal, not the string
/// representation. `None` values (unparseable decimals) are considered less than
/// any valid decimal value.
impl PartialOrd for PreciseDecimal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

/// Implements total ordering for `PreciseDecimal`.
///
/// Provides a consistent ordering for sorting operations. The ordering is based
/// on the mathematical value: `None` < `Some(smaller_decimal)` < `Some(larger_decimal)`.
impl Ord for PreciseDecimal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

// === PreciseDecimal Methods ===

impl PreciseDecimal {
    /// Creates a new `PreciseDecimal` from its constituent parts.
    ///
    /// This constructor allows explicit control over both the parsed value and the
    /// original string representation. Use this when you need to preserve a specific
    /// string format or when parsing has already been attempted.
    ///
    /// # Arguments
    ///
    /// * `value` - The parsed decimal value, or `None` if parsing failed
    /// * `original_string` - The original string representation to preserve
    ///
    /// # Examples
    ///
    /// ```rust
    /// use fhir::PreciseDecimal;
    /// use rust_decimal::Decimal;
    ///
    /// // Create with successful parsing
    /// let precise = PreciseDecimal::from_parts(
    ///     Some(Decimal::new(12340, 3)),
    ///     "12.340".to_string()
    /// );
    ///
    /// // Create with failed parsing (preserves original string)
    /// let invalid = PreciseDecimal::from_parts(
    ///     None,
    ///     "invalid_decimal".to_string()
    /// );
    /// ```
    pub fn from_parts(value: Option<Decimal>, original_string: String) -> Self {
        Self {
            value,
            original_string,
        }
    }

    /// Helper method to parse a decimal string with support for scientific notation.
    ///
    /// This method handles the complexity of parsing decimal strings that may be in
    /// scientific notation (with 'E' or 'e' exponents) or regular decimal format.
    /// It normalizes 'E' to 'e' for consistent parsing while preserving the original
    /// string representation for serialization.
    ///
    /// # Arguments
    ///
    /// * `s` - The string to parse as a decimal
    ///
    /// # Returns
    ///
    /// `Some(Decimal)` if parsing succeeds, `None` if the string is not a valid decimal.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use fhir::PreciseDecimal;
    /// use rust_decimal::Decimal;
    ///
    /// // Regular decimal format
    /// assert!(PreciseDecimal::parse_decimal_string("123.45").is_some());
    ///
    /// // Scientific notation with 'e'
    /// assert!(PreciseDecimal::parse_decimal_string("1.23e2").is_some());
    ///
    /// // Scientific notation with 'E' (normalized to 'e')
    /// assert!(PreciseDecimal::parse_decimal_string("1.23E2").is_some());
    ///
    /// // Invalid format
    /// assert!(PreciseDecimal::parse_decimal_string("invalid").is_none());
    /// ```
    fn parse_decimal_string(s: &str) -> Option<Decimal> {
        // Normalize 'E' to 'e' for consistent parsing
        let normalized = s.replace('E', "e");
        
        if normalized.contains('e') {
            // Use scientific notation parsing
            Decimal::from_scientific(&normalized).ok()
        } else {
            // Use regular decimal parsing
            normalized.parse::<Decimal>().ok()
        }
    }

    /// Returns the parsed decimal value if parsing was successful.
    ///
    /// This method provides access to the mathematical value for arithmetic
    /// operations and comparisons. Returns `None` if the original string
    /// could not be parsed as a valid decimal.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use fhir::PreciseDecimal;
    /// use rust_decimal::Decimal;
    ///
    /// let precise = PreciseDecimal::from(Decimal::new(1234, 2)); // 12.34
    /// assert_eq!(precise.value(), Some(Decimal::new(1234, 2)));
    ///
    /// let invalid = PreciseDecimal::from_parts(None, "invalid".to_string());
    /// assert_eq!(invalid.value(), None);
    /// ```
    pub fn value(&self) -> Option<Decimal> {
        self.value
    }

    /// Returns the original string representation.
    ///
    /// This method provides access to the exact string format that was used
    /// to create this `PreciseDecimal`. This string is used during serialization
    /// to maintain FHIR's precision requirements.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use fhir::PreciseDecimal;
    /// use rust_decimal::Decimal;
    ///
    /// let precise = PreciseDecimal::from_parts(
    ///     Some(Decimal::new(100, 2)),
    ///     "1.00".to_string()
    /// );
    /// assert_eq!(precise.original_string(), "1.00");
    /// ```
    pub fn original_string(&self) -> &str {
        &self.original_string
    }
}

/// Converts a `Decimal` to `PreciseDecimal` with derived string representation.
///
/// This implementation allows easy conversion from `rust_decimal::Decimal` values
/// by automatically generating the string representation using the decimal's
/// `Display` implementation.
///
/// # Examples
///
/// ```rust
/// use fhir::PreciseDecimal;
/// use rust_decimal::Decimal;
///
/// let decimal = Decimal::new(12345, 3); // 12.345
/// let precise: PreciseDecimal = decimal.into();
/// assert_eq!(precise.value(), Some(decimal));
/// assert_eq!(precise.original_string(), "12.345");
/// ```
impl From<Decimal> for PreciseDecimal {
    fn from(value: Decimal) -> Self {
        // Generate string representation from the decimal value
        let original_string = value.to_string();
        Self {
            value: Some(value),
            original_string,
        }
    }
}

/// Implements serialization for `PreciseDecimal` preserving original format.
///
/// This implementation ensures that the exact original string representation
/// is preserved during JSON serialization, maintaining FHIR's precision
/// requirements including trailing zeros and specific formatting.
///
/// # FHIR Compliance
///
/// FHIR requires that decimal values maintain their original precision when
/// round-tripped through JSON. This implementation uses `serde_json::RawValue`
/// to serialize the original string directly as a JSON number.
///
/// # Examples
///
/// ```rust
/// use fhir::PreciseDecimal;
/// use rust_decimal::Decimal;
/// use serde_json;
///
/// let precise = PreciseDecimal::from_parts(
///     Some(Decimal::new(1230, 2)),
///     "12.30".to_string()
/// );
/// 
/// let json = serde_json::to_string(&precise).unwrap();
/// assert_eq!(json, "12.30"); // Preserves trailing zero
/// ```
impl Serialize for PreciseDecimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Use RawValue to preserve exact string format in JSON
        match serde_json::value::RawValue::from_string(self.original_string.clone()) {
            Ok(raw_value) => raw_value.serialize(serializer),
            Err(e) => Err(serde::ser::Error::custom(format!(
                "Failed to serialize PreciseDecimal '{}': {}",
                self.original_string, e
            ))),
        }
    }
}

/// Implements deserialization for `PreciseDecimal` preserving original format.
///
/// This implementation deserializes JSON numbers and strings into `PreciseDecimal`
/// while preserving the exact original string representation. It handles various
/// JSON formats including scientific notation and nested object structures.
///
/// # Supported Formats
///
/// - Direct numbers: `12.340`
/// - String numbers: `"12.340"`
/// - Scientific notation: `1.234e2` or `1.234E2`
/// - Nested objects: `{"value": 12.340}` (for macro-generated structures)
///
/// # Examples
///
/// ```rust
/// use fhir::PreciseDecimal;
/// use serde_json;
///
/// // Deserialize from JSON number
/// let precise: PreciseDecimal = serde_json::from_str("12.340").unwrap();
/// assert_eq!(precise.original_string(), "12.34"); // JSON number format
///
/// // Deserialize from JSON string
/// let precise: PreciseDecimal = serde_json::from_str("\"12.340\"").unwrap();
/// assert_eq!(precise.original_string(), "12.340"); // Preserves string format
/// ```
impl<'de> Deserialize<'de> for PreciseDecimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Use intermediate Value to capture exact string representation
        let json_value = serde_json::Value::deserialize(deserializer)?;

        match json_value {
            serde_json::Value::Number(n) => {
                // Extract string representation from JSON number
                let original_string = n.to_string();
                let parsed_value = Self::parse_decimal_string(&original_string);
                Ok(PreciseDecimal::from_parts(parsed_value, original_string))
            }
            serde_json::Value::String(s) => {
                // Use string value directly (preserves exact format)
                let parsed_value = Self::parse_decimal_string(&s);
                Ok(PreciseDecimal::from_parts(parsed_value, s))
            }
            // Handle nested object format (for macro-generated structures)
            serde_json::Value::Object(map) => {
                match map.get("value") {
                    Some(serde_json::Value::Number(n)) => {
                        let original_string = n.to_string();
                        let parsed_value = Self::parse_decimal_string(&original_string);
                        Ok(PreciseDecimal::from_parts(parsed_value, original_string))
                    }
                    Some(serde_json::Value::String(s)) => {
                        let original_string = s.clone();
                        let parsed_value = Self::parse_decimal_string(&original_string);
                        Ok(PreciseDecimal::from_parts(parsed_value, original_string))
                    }
                    Some(serde_json::Value::Null) => {
                        Err(de::Error::invalid_value(
                            de::Unexpected::Unit,
                            &"a number or string for decimal value",
                        ))
                    }
                    None => Err(de::Error::missing_field("value")),
                    _ => Err(de::Error::invalid_type(
                        de::Unexpected::Map,
                        &"a map with a 'value' field containing a number or string",
                    )),
                }
            }
            // Handle remaining unexpected types
            other => Err(de::Error::invalid_type(
                match other {
                    serde_json::Value::Null => de::Unexpected::Unit, // Or Unexpected::Option if mapping null to None
                    serde_json::Value::Bool(b) => de::Unexpected::Bool(b),
                    serde_json::Value::Array(_) => de::Unexpected::Seq,
                    _ => de::Unexpected::Other("unexpected JSON type for PreciseDecimal"),
                },
                &"a number, string, or object with a 'value' field",
            )),
        }
    }
}

// --- End PreciseDecimal ---

// Removed DecimalElementObjectVisitor

#[cfg(feature = "R4")]
pub mod r4;
#[cfg(feature = "R4B")]
pub mod r4b;
#[cfg(feature = "R5")]
pub mod r5;
#[cfg(feature = "R6")]
pub mod r6;

// Removed the FhirSerde trait definition

/// Multi-version FHIR resource container supporting version-agnostic operations.
///
/// This enum provides a unified interface for working with FHIR resources across
/// different specification versions. It enables applications to handle multiple
/// FHIR versions simultaneously while maintaining type safety and version-specific
/// behavior where needed.
///
/// # Supported Versions
///
/// - **R4**: FHIR 4.0.1 (normative)
/// - **R4B**: FHIR 4.3.0 (ballot)  
/// - **R5**: FHIR 5.0.0 (ballot)
/// - **R6**: FHIR 6.0.0 (draft)
///
/// # Feature Flags
///
/// Each FHIR version is controlled by a corresponding Cargo feature flag.
/// Only enabled versions will be available in the enum variants.
///
/// # Examples
///
/// ```rust
/// use fhir::{FhirResource, FhirVersion};
/// # #[cfg(feature = "R5")]
/// use fhir::r5::{Patient, HumanName};
///
/// # #[cfg(feature = "R5")]
/// {
///     // Create an R5 patient
///     let patient = Patient {
///         name: Some(vec![HumanName {
///             family: Some("Doe".to_string()),
///             given: Some(vec!["John".to_string()]),
///             ..Default::default()
///         }]),
///         ..Default::default()
///     };
///
///     // Wrap in version-agnostic container
///     let resource = FhirResource::R5(Box::new(patient.into()));
///     assert_eq!(resource.version(), FhirVersion::R5);
/// }
/// ```
///
/// # Version Detection
///
/// Use the `version()` method to determine which FHIR version a resource uses:
///
/// ```rust
/// # use fhir::{FhirResource, FhirVersion};
/// # #[cfg(feature = "R4")]
/// # {
/// # let resource = FhirResource::R4(Box::new(fhir::r4::Resource::Patient(Default::default())));
/// match resource.version() {
///     FhirVersion::R4 => println!("This is an R4 resource"),
///     FhirVersion::R4B => println!("This is an R4B resource"),
///     FhirVersion::R5 => println!("This is an R5 resource"),
///     FhirVersion::R6 => println!("This is an R6 resource"),
/// }
/// # }
/// ```
#[derive(Debug)]
pub enum FhirResource {
    /// FHIR 4.0.1 (normative) resource
    #[cfg(feature = "R4")]
    R4(Box<r4::Resource>),
    /// FHIR 4.3.0 (ballot) resource
    #[cfg(feature = "R4B")]
    R4B(Box<r4b::Resource>),
    /// FHIR 5.0.0 (ballot) resource
    #[cfg(feature = "R5")]
    R5(Box<r5::Resource>),
    /// FHIR 6.0.0 (draft) resource
    #[cfg(feature = "R6")]
    R6(Box<r6::Resource>),
}

impl FhirResource {
    /// Returns the FHIR specification version of this resource.
    ///
    /// This method provides version detection for multi-version applications,
    /// enabling version-specific processing logic and compatibility checks.
    ///
    /// # Returns
    ///
    /// The `FhirVersion` enum variant corresponding to this resource's specification.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use fhir::{FhirResource, FhirVersion};
    /// 
    /// # #[cfg(feature = "R5")]
    /// # {
    /// # let resource = FhirResource::R5(Box::new(fhir::r5::Resource::Patient(Default::default())));
    /// let version = resource.version();
    /// assert_eq!(version, FhirVersion::R5);
    /// 
    /// // Use version for conditional logic
    /// match version {
    ///     FhirVersion::R5 => {
    ///         println!("Processing R5 resource with latest features");
    ///     },
    ///     FhirVersion::R4 => {
    ///         println!("Processing R4 resource with normative features");
    ///     },
    ///     _ => {
    ///         println!("Processing other FHIR version");
    ///     }
    /// }
    /// # }
    /// ```
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

/// Enumeration of supported FHIR specification versions.
///
/// This enum represents the different versions of the FHIR (Fast Healthcare
/// Interoperability Resources) specification that this library supports.
/// Each version represents a specific release of the FHIR standard with
/// its own set of features, resources, and compatibility requirements.
///
/// # Version Status
///
/// - **R4** (4.0.1): Normative version, widely adopted in production
/// - **R4B** (4.3.0): Ballot version with additional features
/// - **R5** (5.0.0): Ballot version with significant enhancements
/// - **R6** (6.0.0): Draft version under active development
///
/// # Feature Flags
///
/// Each version is controlled by a corresponding Cargo feature flag:
/// - `R4`: Enables FHIR R4 support
/// - `R4B`: Enables FHIR R4B support  
/// - `R5`: Enables FHIR R5 support
/// - `R6`: Enables FHIR R6 support
///
/// # Examples
///
/// ```rust
/// use fhir::FhirVersion;
///
/// // Version comparison
/// # #[cfg(all(feature = "R4", feature = "R5"))]
/// # {
/// assert_ne!(FhirVersion::R4, FhirVersion::R5);
/// # }
///
/// // String representation
/// # #[cfg(feature = "R4")]
/// # {
/// let version = FhirVersion::R4;
/// assert_eq!(version.as_str(), "R4");
/// assert_eq!(version.to_string(), "R4");
/// # }
/// ```
///
/// # CLI Integration
///
/// This enum implements `clap::ValueEnum` for command-line argument parsing:
///
/// ```rust,no_run
/// use clap::Parser;
/// use fhir::FhirVersion;
///
/// #[derive(Parser)]
/// struct Args {
///     #[arg(value_enum)]
///     version: FhirVersion,
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FhirVersion {
    /// FHIR 4.0.1 (normative) - The current normative version
    #[cfg(feature = "R4")]
    R4,
    /// FHIR 4.3.0 (ballot) - Intermediate version with additional features
    #[cfg(feature = "R4B")]
    R4B,
    /// FHIR 5.0.0 (ballot) - Next major version with significant changes
    #[cfg(feature = "R5")]
    R5,
    /// FHIR 6.0.0 (draft) - Future version under development
    #[cfg(feature = "R6")]
    R6,
}

impl FhirVersion {
    /// Returns the string representation of the FHIR version.
    ///
    /// This method provides the standard version identifier as used in
    /// FHIR documentation, URLs, and configuration files.
    ///
    /// # Returns
    ///
    /// A static string slice representing the version (e.g., "R4", "R5").
    ///
    /// # Examples
    ///
    /// ```rust
    /// use fhir::FhirVersion;
    ///
    /// # #[cfg(feature = "R4")]
    /// assert_eq!(FhirVersion::R4.as_str(), "R4");
    /// # #[cfg(feature = "R5")]
    /// assert_eq!(FhirVersion::R5.as_str(), "R5");
    /// ```
    ///
    /// # Usage
    ///
    /// This method is commonly used for:
    /// - Logging and debugging output
    /// - Configuration file parsing
    /// - API endpoint construction
    /// - Version-specific resource loading
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

/// Implements `Display` trait for user-friendly output formatting.
///
/// This enables `FhirVersion` to be used in string formatting operations
/// and provides consistent output across different contexts.
///
/// # Examples
///
/// ```rust
/// use fhir::FhirVersion;
///
/// # #[cfg(feature = "R5")]
/// # {
/// let version = FhirVersion::R5;
/// println!("Using FHIR version: {}", version); // Prints: "Using FHIR version: R5"
/// 
/// let formatted = format!("fhir-{}.json", version);
/// assert_eq!(formatted, "fhir-R5.json");
/// # }
/// ```
impl std::fmt::Display for FhirVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Provides a default FHIR version when R4 feature is enabled.
///
/// R4 is chosen as the default because it is the current normative version
/// of the FHIR specification and is widely adopted in production systems.
///
/// # Examples
///
/// ```rust
/// use fhir::FhirVersion;
///
/// # #[cfg(feature = "R4")]
/// # {
/// let default_version = FhirVersion::default();
/// assert_eq!(default_version, FhirVersion::R4);
/// # }
/// ```
#[cfg(feature = "R4")]
impl Default for FhirVersion {
    fn default() -> Self {
        FhirVersion::R4
    }
}

/// Implements `clap::ValueEnum` for command-line argument parsing.
///
/// This implementation enables `FhirVersion` to be used directly as a command-line
/// argument type with clap, providing automatic parsing, validation, and help text
/// generation.
///
/// # Examples
///
/// ```rust,no_run
/// use clap::Parser;
/// use fhir::FhirVersion;
///
/// #[derive(Parser)]
/// struct Args {
///     /// FHIR specification version to use
///     #[arg(value_enum, default_value_t = FhirVersion::default())]
///     version: FhirVersion,
/// }
///
/// // Command line: my-app --version R5
/// let args = Args::parse();
/// println!("Using FHIR version: {}", args.version);
/// ```
///
/// # Generated Help Text
///
/// When using this enum with clap, the help text will automatically include
/// all available FHIR versions based on enabled feature flags.
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

/// Trait for providing FHIR resource type information
/// 
/// This trait allows querying which resource types are available in a specific
/// FHIR version without hardcoding resource type lists in multiple places.
pub trait FhirResourceTypeProvider {
    /// Returns a vector of all resource type names supported in this FHIR version
    fn get_resource_type_names() -> Vec<&'static str>;
    
    /// Checks if a given type name is a resource type in this FHIR version
    fn is_resource_type(type_name: &str) -> bool {
        Self::get_resource_type_names()
            .iter()
            .any(|&resource_type| resource_type.eq_ignore_ascii_case(type_name))
    }
}

// --- Internal Visitor for Element Object Deserialization ---

/// Internal visitor struct for deserializing Element objects from JSON maps.
/// 
/// This visitor handles the complex deserialization logic for Element<V, E> when
/// the JSON input is an object containing id, extension, and value fields.
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

/// Generic element container supporting FHIR's extension mechanism.
///
/// In FHIR, most primitive elements can be extended with additional metadata
/// through the `id` and `extension` fields. This container type provides
/// the infrastructure to support this pattern across all FHIR data types.
///
/// # Type Parameters
///
/// * `V` - The value type (e.g., `String`, `i32`, `PreciseDecimal`)
/// * `E` - The extension type (typically the generated `Extension` struct)
///
/// # FHIR Element Structure
///
/// FHIR elements can appear in three forms:
/// 1. **Primitive value**: Just the value itself (e.g., `"text"`, `42`)
/// 2. **Extended primitive**: An object with `value`, `id`, and/or `extension` fields
/// 3. **Extension-only**: An object with just `id` and/or `extension` (no value)
///
/// # Examples
///
/// ```rust
/// use fhir::{Element, r5::Extension};
///
/// // Simple primitive value
/// let simple: Element<String, Extension> = Element {
///     value: Some("Hello World".to_string()),
///     id: None,
///     extension: None,
/// };
///
/// // Extended primitive with ID
/// let with_id: Element<String, Extension> = Element {
///     value: Some("Hello World".to_string()),
///     id: Some("text-element-1".to_string()),
///     extension: None,
/// };
///
/// // Extension-only element (no value)
/// let extension_only: Element<String, Extension> = Element {
///     value: None,
///     id: Some("disabled-element".to_string()),
///     extension: Some(vec![/* extensions */]),
/// };
/// ```
///
/// # Serialization Behavior
///
/// - If only `value` is present: serializes as the primitive value directly
/// - If `id` or `extension` are present: serializes as an object with all fields
/// - If everything is `None`: serializes as `null`
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct Element<V, E> {
    /// Optional element identifier for referencing within the resource
    pub id: Option<String>,
    /// Optional extensions providing additional metadata
    pub extension: Option<Vec<E>>,
    /// The actual primitive value
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

/// Specialized element container for FHIR decimal values with precision preservation.
///
/// This type combines the generic `Element` pattern with `PreciseDecimal` to provide
/// a complete solution for FHIR decimal elements that require both extension support
/// and precision preservation during serialization round-trips.
///
/// # Type Parameters
///
/// * `E` - The extension type (typically the generated `Extension` struct)
///
/// # FHIR Decimal Requirements
///
/// FHIR decimal elements must:
/// - Preserve original string precision (e.g., "12.30" vs "12.3")
/// - Support mathematical operations using `Decimal` arithmetic
/// - Handle extension metadata through `id` and `extension` fields
/// - Serialize back to the exact original format when possible
///
/// # Examples
///
/// ```rust
/// use fhir::{DecimalElement, PreciseDecimal, r5::Extension};
/// use rust_decimal::Decimal;
///
/// // Create from a Decimal value
/// let decimal_elem = DecimalElement::<Extension>::new(Decimal::new(1234, 2)); // 12.34
///
/// // Create with extensions
/// let extended_decimal = DecimalElement {
///     value: Some(PreciseDecimal::from_parts(
///         Some(Decimal::new(12300, 3)),
///         "12.300".to_string()
///     )),
///     id: Some("precision-example".to_string()),
///     extension: Some(vec![/* extensions */]),
/// };
///
/// // Access the mathematical value
/// if let Some(precise) = &extended_decimal.value {
///     if let Some(decimal_val) = precise.value() {
///         println!("Mathematical value: {}", decimal_val);
///     }
///     println!("Original format: {}", precise.original_string());
/// }
/// ```
///
/// # Serialization Behavior
///
/// - **Value only**: Serializes as a JSON number preserving original precision
/// - **With extensions**: Serializes as an object with `value`, `id`, and `extension` fields
/// - **No value**: Serializes as an object with just the extension fields, or `null` if empty
///
/// # Integration with FHIRPath
///
/// When used with FHIRPath evaluation, `DecimalElement` returns:
/// - The `Decimal` value for mathematical operations
/// - An object representation when extension metadata is accessed
/// - Empty collection when the element has no value or extensions
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct DecimalElement<E> {
    /// Optional element identifier for referencing within the resource
    pub id: Option<String>,
    /// Optional extensions providing additional metadata
    pub extension: Option<Vec<E>>,
    /// The decimal value with precision preservation
    pub value: Option<PreciseDecimal>,
}

impl<E> DecimalElement<E> {
    /// Creates a new `DecimalElement` with the specified decimal value.
    ///
    /// This constructor creates a simple decimal element with no extensions or ID,
    /// containing only the decimal value. The original string representation is
    /// automatically derived from the `Decimal` value's `Display` implementation.
    ///
    /// # Arguments
    ///
    /// * `value` - The `Decimal` value to store
    ///
    /// # Returns
    ///
    /// A new `DecimalElement` with the value set and `id`/`extension` as `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use fhir::{DecimalElement, r5::Extension};
    /// use rust_decimal::Decimal;
    ///
    /// // Create a simple decimal element
    /// let element = DecimalElement::<Extension>::new(Decimal::new(12345, 3)); // 12.345
    ///
    /// // Verify the structure
    /// assert!(element.id.is_none());
    /// assert!(element.extension.is_none());
    /// assert!(element.value.is_some());
    ///
    /// // Access the decimal value
    /// if let Some(precise_decimal) = &element.value {
    ///     assert_eq!(precise_decimal.value(), Some(Decimal::new(12345, 3)));
    ///     assert_eq!(precise_decimal.original_string(), "12.345");
    /// }
    /// ```
    ///
    /// # Usage in FHIR Resources
    ///
    /// This method is typically used when creating FHIR elements programmatically:
    ///
    /// ```rust
    /// use fhir::{DecimalElement, r5::{Extension, Observation}};
    /// use rust_decimal::Decimal;
    ///
    /// let temperature = DecimalElement::<Extension>::new(Decimal::new(3672, 2)); // 36.72
    /// 
    /// // Would be used in an Observation like:
    /// // observation.value_quantity.value = Some(temperature);
    /// ```
    pub fn new(value: Decimal) -> Self {
        // Convert the Decimal to PreciseDecimal, which automatically handles
        // storing the original string representation via the From trait
        let precise_value = PreciseDecimal::from(value);
        Self {
            id: None,
            extension: None,
            value: Some(precise_value),
        }
    }
}

// Custom Deserialize for DecimalElement<E> using intermediate Value
impl<'de, E> Deserialize<'de> for DecimalElement<E>
where
    E: Deserialize<'de> + Default,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize into an intermediate serde_json::Value first
        let json_value = serde_json::Value::deserialize(deserializer)?;

        match json_value {
            // Handle primitive JSON Number
            serde_json::Value::Number(n) => {
                // Directly parse the number string to create PreciseDecimal
                let s = n.to_string(); // Note: n.to_string() might normalize exponent case (e.g., 'E' -> 'e')
                // Replace 'E' with 'e' for parsing
                let s_for_parsing = s.replace('E', "e");
                // Use from_scientific if 'e' is present, otherwise parse
                let parsed_value = if s_for_parsing.contains('e') {
                    Decimal::from_scientific(&s_for_parsing).ok()
                } else {
                    s_for_parsing.parse::<Decimal>().ok()
                };
                // Store the ORIGINAL string `s` (as returned by n.to_string()).
                let pd = PreciseDecimal::from_parts(parsed_value, s);
                Ok(DecimalElement {
                    id: None,
                    extension: None,
                    value: Some(pd),
                })
            }
            // Handle primitive JSON String
            serde_json::Value::String(s) => {
                // Directly parse the string to create PreciseDecimal
                // Replace 'E' with 'e' for parsing
                let s_for_parsing = s.replace('E', "e");
                // Use from_scientific if 'e' is present, otherwise parse
                let parsed_value = if s_for_parsing.contains('e') {
                    Decimal::from_scientific(&s_for_parsing).ok()
                } else {
                    s_for_parsing.parse::<Decimal>().ok()
                };
                // Store the ORIGINAL string `s`.
                let pd = PreciseDecimal::from_parts(parsed_value, s); // s is owned, no clone needed
                Ok(DecimalElement {
                    id: None,
                    extension: None,
                    value: Some(pd),
                })
            }
            // Handle JSON object: deserialize fields individually
            serde_json::Value::Object(map) => {
                let mut id: Option<String> = None;
                let mut extension: Option<Vec<E>> = None;
                let mut value: Option<PreciseDecimal> = None;

                for (k, v) in map {
                    match k.as_str() {
                        "id" => {
                            if id.is_some() {
                                return Err(de::Error::duplicate_field("id"));
                            }
                            // Deserialize id directly from its Value
                            id = Deserialize::deserialize(v).map_err(de::Error::custom)?;
                        }
                        "extension" => {
                            if extension.is_some() {
                                return Err(de::Error::duplicate_field("extension"));
                            }
                            // Deserialize extension directly from its Value
                            extension = Deserialize::deserialize(v).map_err(de::Error::custom)?;
                        }
                        "value" => {
                            if value.is_some() {
                                return Err(de::Error::duplicate_field("value"));
                            }
                            // Deserialize value using PreciseDecimal::deserialize from its Value
                            // Handle null explicitly within the value field
                            if v.is_null() {
                                value = None;
                            } else {
                                value = Some(
                                    PreciseDecimal::deserialize(v).map_err(de::Error::custom)?,
                                );
                            }
                        }
                        // Ignore any unknown fields encountered
                        _ => {} // Simply ignore unknown fields
                    }
                }
                Ok(DecimalElement {
                    id,
                    extension,
                    value,
                })
            }
            // Handle JSON Null for the whole element
            serde_json::Value::Null => Ok(DecimalElement::default()), // Default has value: None
            // Handle other unexpected types
            other => Err(de::Error::invalid_type(
                match other {
                    serde_json::Value::Bool(b) => de::Unexpected::Bool(b),
                    serde_json::Value::Array(_) => de::Unexpected::Seq,
                    _ => de::Unexpected::Other("unexpected JSON type for DecimalElement"),
                },
                &"a decimal number, string, object, or null",
            )),
        }
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

// For Element<V, E> - Returns Object with id, extension, value if present
impl<V, E> IntoEvaluationResult for Element<V, E>
where
    V: IntoEvaluationResult + Clone + 'static,
    E: IntoEvaluationResult + Clone,
{
    fn into_evaluation_result(&self) -> EvaluationResult {
        use std::any::TypeId;
        
        // Prioritize returning the primitive value if it exists
        if let Some(v) = &self.value {
            let result = v.into_evaluation_result();
            // For primitive values, we need to preserve FHIR type information
            return match result {
                EvaluationResult::Boolean(b, _) => {
                    // Return FHIR boolean
                    EvaluationResult::fhir_boolean(b)
                }
                EvaluationResult::Integer(i, _) => {
                    // Return FHIR integer
                    EvaluationResult::fhir_integer(i)
                }
                EvaluationResult::String(s, _) => {
                    // Determine the FHIR type name based on V's type
                    let fhir_type_name = if TypeId::of::<V>() == TypeId::of::<String>() {
                        // For strings, we need more context to determine the exact FHIR type
                        // Default to "string" but this could be date, dateTime, etc.
                        "string"
                    } else {
                        // Default fallback
                        "string"
                    };
                    EvaluationResult::fhir_string(s, fhir_type_name)
                }
                _ => result, // For other types, return as-is
            };
        } else if self.id.is_some() || self.extension.is_some() {
            // If value is None, but id or extension exist, return an Object with those
            let mut map = std::collections::HashMap::new();
            if let Some(id) = &self.id {
                map.insert("id".to_string(), EvaluationResult::string(id.clone()));
            }
            if let Some(ext) = &self.extension {
                let ext_collection: Vec<EvaluationResult> = ext
                    .iter()
                    .map(|e| e.into_evaluation_result())
                    .collect();
                if !ext_collection.is_empty() {
                    map.insert("extension".to_string(), EvaluationResult::collection(ext_collection));
                }
            }
            // Only return Object if map is not empty (i.e., id or extension was actually present)
            if !map.is_empty() {
                return EvaluationResult::typed_object(map, "FHIR", "Element");
            }
        }

        // If value, id, and extension are all None, return Empty
        EvaluationResult::Empty
    }
}

// For DecimalElement<E> - Returns Decimal value if present, otherwise handles id/extension
impl<E> IntoEvaluationResult for DecimalElement<E>
where
    E: IntoEvaluationResult + Clone,
{
    fn into_evaluation_result(&self) -> EvaluationResult {
        // Prioritize returning the primitive decimal value if it exists
        if let Some(precise_decimal) = &self.value {
            if let Some(decimal_val) = precise_decimal.value() {
                // Return FHIR decimal
                return EvaluationResult::fhir_decimal(decimal_val);
            }
            // If PreciseDecimal holds None for value, fall through to check id/extension
        }

        // If value is None, but id or extension exist, return an Object with those
        if self.id.is_some() || self.extension.is_some() {
            let mut map = std::collections::HashMap::new();
            if let Some(id) = &self.id {
                map.insert("id".to_string(), EvaluationResult::string(id.clone()));
            }
            if let Some(ext) = &self.extension {
                let ext_collection: Vec<EvaluationResult> = ext
                    .iter()
                    .map(|e| e.into_evaluation_result())
                    .collect();
                if !ext_collection.is_empty() {
                    map.insert("extension".to_string(), EvaluationResult::collection(ext_collection));
                }
            }
             // Only return Object if map is not empty
            if !map.is_empty() {
                return EvaluationResult::typed_object(map, "FHIR", "decimal");
            }
        }

        // If value, id, and extension are all None, return Empty
        EvaluationResult::Empty
    }
}

// Implement the trait for the top-level enum
impl IntoEvaluationResult for FhirResource {
    fn into_evaluation_result(&self) -> EvaluationResult {
        match self {
            #[cfg(feature = "R4")]
            FhirResource::R4(r) => (*r).into_evaluation_result(), // Call impl on inner Box<r4::Resource>
            #[cfg(feature = "R4B")]
            FhirResource::R4B(r) => (*r).into_evaluation_result(), // Call impl on inner Box<r4b::Resource>
            #[cfg(feature = "R5")]
            FhirResource::R5(r) => (*r).into_evaluation_result(), // Call impl on inner Box<r5::Resource>
            #[cfg(feature = "R6")]
            FhirResource::R6(r) => (*r).into_evaluation_result(), // Call impl on inner Box<r6::Resource>
                                                                  // Note: If no features are enabled, this match might be empty or non-exhaustive.
                                                                  // This is generally okay as the enum itself wouldn't be usable.
        }
    }
}
