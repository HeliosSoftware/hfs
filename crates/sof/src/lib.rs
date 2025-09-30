//! # SQL-on-FHIR Implementation
//!
//! This crate provides a complete implementation of the [SQL-on-FHIR
//! specification](https://sql-on-fhir.org/ig/latest),
//! enabling the transformation of FHIR resources into tabular data using declarative
//! ViewDefinitions. It supports all major FHIR versions (R4, R4B, R5, R6) through
//! a version-agnostic abstraction layer.

//!
//! There are three consumers of this crate:
//! - [sof_cli](../sof_cli/index.html) - A command-line interface for the SQL-on-FHIR implementation,
//!   allowing users to execute ViewDefinition transformations on FHIR Bundle resources
//!   and output the results in various formats.
//! - [sof_server](../sof_server/index.html) - A stateless HTTP server implementation for the SQL-on-FHIR specification,
//!   enabling HTTP-based access to ViewDefinition transformation capabilities.
//! - [hfs](../hfs/index.html) - The full featured Helios FHIR Server.
//!
//! ## Architecture
//!
//! The SOF crate is organized around these key components:
//! - **Version-agnostic enums** ([`SofViewDefinition`], [`SofBundle`]): Multi-version containers
//! - **Processing engine** ([`run_view_definition`]): Core transformation logic
//! - **Output formats** ([`ContentType`]): Support for CSV, JSON, NDJSON, and Parquet
//! - **Trait abstractions** ([`ViewDefinitionTrait`], [`BundleTrait`]): Version independence
//!
//! ## Key Features
//!
//! - **Multi-version FHIR support**: Works with R4, R4B, R5, and R6 resources
//! - **FHIRPath evaluation**: Complex path expressions for data extraction
//! - **forEach iteration**: Supports flattening of nested FHIR structures
//! - **unionAll operations**: Combines multiple select statements
//! - **Collection handling**: Proper array serialization for multi-valued fields
//! - **Output formats**: CSV (with/without headers), JSON, NDJSON, Parquet support
//!
//! ## Usage Example
//!
//! ```rust
//! use helios_sof::{SofViewDefinition, SofBundle, ContentType, run_view_definition};
//! use helios_fhir::FhirVersion;
//!
//! # #[cfg(feature = "R4")]
//! # {
//! // Parse a ViewDefinition and Bundle from JSON
//! let view_definition_json = r#"{
//!     "resourceType": "ViewDefinition",
//!     "status": "active",
//!     "resource": "Patient",
//!     "select": [{
//!         "column": [{
//!             "name": "id",
//!             "path": "id"
//!         }, {
//!             "name": "name",
//!             "path": "name.family"
//!         }]
//!     }]
//! }"#;
//!
//! let bundle_json = r#"{
//!     "resourceType": "Bundle",
//!     "type": "collection",
//!     "entry": [{
//!         "resource": {
//!             "resourceType": "Patient",
//!             "id": "example",
//!             "name": [{
//!                 "family": "Doe",
//!                 "given": ["John"]
//!             }]
//!         }
//!     }]
//! }"#;
//!
//! let view_definition: helios_fhir::r4::ViewDefinition = serde_json::from_str(view_definition_json)?;
//! let bundle: helios_fhir::r4::Bundle = serde_json::from_str(bundle_json)?;
//!
//! // Wrap in version-agnostic containers
//! let sof_view = SofViewDefinition::R4(view_definition);
//! let sof_bundle = SofBundle::R4(bundle);
//!
//! // Transform to CSV with headers
//! let csv_output = run_view_definition(
//!     sof_view,
//!     sof_bundle,
//!     ContentType::CsvWithHeader
//! )?;
//!
//! // Check the CSV output
//! let csv_string = String::from_utf8(csv_output)?;
//! assert!(csv_string.contains("id,name"));
//! // CSV values are quoted
//! assert!(csv_string.contains("example") && csv_string.contains("Doe"));
//! # }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Advanced Features
//!
//! ### forEach Iteration
//!
//! ViewDefinitions can iterate over collections using `forEach` and `forEachOrNull`:
//!
//! ```json
//! {
//!   "select": [{
//!     "forEach": "name",
//!     "column": [{
//!       "name": "family_name",
//!       "path": "family"
//!     }]
//!   }]
//! }
//! ```
//!
//! ### Constants and Variables
//!
//! Define reusable values in ViewDefinitions:
//!
//! ```json
//! {
//!   "constant": [{
//!     "name": "system",
//!     "valueString": "http://loinc.org"
//!   }],
//!   "select": [{
//!     "where": [{
//!       "path": "code.coding.system = %system"
//!     }]
//!   }]
//! }
//! ```
//!
//! ### Where Clauses
//!
//! Filter resources using FHIRPath expressions:
//!
//! ```json
//! {
//!   "where": [{
//!     "path": "active = true"
//!   }, {
//!     "path": "birthDate.exists()"
//!   }]
//! }
//! ```
//!
//! ## Error Handling
//!
//! The crate provides comprehensive error handling through [`SofError`]:
//!
//! ```rust,no_run
//! use helios_sof::{SofError, SofViewDefinition, SofBundle, ContentType, run_view_definition};
//!
//! # let view = SofViewDefinition::R4(helios_fhir::r4::ViewDefinition::default());
//! # let bundle = SofBundle::R4(helios_fhir::r4::Bundle::default());
//! # let content_type = ContentType::Json;
//! match run_view_definition(view, bundle, content_type) {
//!     Ok(output) => {
//!         // Process successful transformation
//!     },
//!     Err(SofError::InvalidViewDefinition(msg)) => {
//!         eprintln!("ViewDefinition validation failed: {}", msg);
//!     },
//!     Err(SofError::FhirPathError(msg)) => {
//!         eprintln!("FHIRPath evaluation failed: {}", msg);
//!     },
//!     Err(e) => {
//!         eprintln!("Other error: {}", e);
//!     }
//! }
//! ```
//! ## Feature Flags
//!
//! Enable support for specific FHIR versions:
//! - `R4`: FHIR 4.0.1 support
//! - `R4B`: FHIR 4.3.0 support
//! - `R5`: FHIR 5.0.0 support
//! - `R6`: FHIR 6.0.0 support

pub mod data_source;
pub mod parquet_schema;
pub mod traits;

use chrono::{DateTime, Utc};
use helios_fhirpath::{EvaluationContext, EvaluationResult, evaluate_expression};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use traits::*;

// Re-export commonly used types and traits for easier access
pub use helios_fhir::FhirVersion;
pub use traits::{BundleTrait, ResourceTrait, ViewDefinitionTrait};

/// Multi-version ViewDefinition container supporting version-agnostic operations.
///
/// This enum provides a unified interface for working with ViewDefinition resources
/// across different FHIR specification versions. It enables applications to handle
/// multiple FHIR versions simultaneously while maintaining type safety.
///
/// # Supported Versions
///
/// - **R4**: FHIR 4.0.1 ViewDefinition (normative)
/// - **R4B**: FHIR 4.3.0 ViewDefinition (ballot)
/// - **R5**: FHIR 5.0.0 ViewDefinition (ballot)
/// - **R6**: FHIR 6.0.0 ViewDefinition (draft)
///
/// # Examples
///
/// ```rust
/// use helios_sof::{SofViewDefinition, ContentType};
/// # #[cfg(feature = "R4")]
/// use helios_fhir::r4::ViewDefinition;
///
/// # #[cfg(feature = "R4")]
/// # {
/// // Parse from JSON
/// let json = r#"{
///     "resourceType": "ViewDefinition",
///     "resource": "Patient",
///     "select": [{
///         "column": [{
///             "name": "id",
///             "path": "id"
///         }]
///     }]
/// }"#;
///
/// let view_def: ViewDefinition = serde_json::from_str(json)?;
/// let sof_view = SofViewDefinition::R4(view_def);
///
/// // Check version
/// assert_eq!(sof_view.version(), helios_fhir::FhirVersion::R4);
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub enum SofViewDefinition {
    #[cfg(feature = "R4")]
    R4(helios_fhir::r4::ViewDefinition),
    #[cfg(feature = "R4B")]
    R4B(helios_fhir::r4b::ViewDefinition),
    #[cfg(feature = "R5")]
    R5(helios_fhir::r5::ViewDefinition),
    #[cfg(feature = "R6")]
    R6(helios_fhir::r6::ViewDefinition),
}

impl SofViewDefinition {
    /// Returns the FHIR specification version of this ViewDefinition.
    ///
    /// This method provides version detection for multi-version applications,
    /// enabling version-specific processing logic and compatibility checks.
    ///
    /// # Returns
    ///
    /// The `FhirVersion` enum variant corresponding to this ViewDefinition's specification.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_sof::SofViewDefinition;
    /// use helios_fhir::FhirVersion;
    ///
    /// # #[cfg(feature = "R5")]
    /// # {
    /// # let view_def = helios_fhir::r5::ViewDefinition::default();
    /// let sof_view = SofViewDefinition::R5(view_def);
    /// assert_eq!(sof_view.version(), helios_fhir::FhirVersion::R5);
    /// # }
    /// ```
    pub fn version(&self) -> helios_fhir::FhirVersion {
        match self {
            #[cfg(feature = "R4")]
            SofViewDefinition::R4(_) => helios_fhir::FhirVersion::R4,
            #[cfg(feature = "R4B")]
            SofViewDefinition::R4B(_) => helios_fhir::FhirVersion::R4B,
            #[cfg(feature = "R5")]
            SofViewDefinition::R5(_) => helios_fhir::FhirVersion::R5,
            #[cfg(feature = "R6")]
            SofViewDefinition::R6(_) => helios_fhir::FhirVersion::R6,
        }
    }
}

/// Multi-version Bundle container supporting version-agnostic operations.
///
/// This enum provides a unified interface for working with FHIR Bundle resources
/// across different FHIR specification versions. Bundles contain the actual FHIR
/// resources that will be processed by ViewDefinitions.
///
/// # Supported Versions
///
/// - **R4**: FHIR 4.0.1 Bundle (normative)
/// - **R4B**: FHIR 4.3.0 Bundle (ballot)
/// - **R5**: FHIR 5.0.0 Bundle (ballot)
/// - **R6**: FHIR 6.0.0 Bundle (draft)
///
/// # Examples
///
/// ```rust
/// use helios_sof::SofBundle;
/// # #[cfg(feature = "R4")]
/// use helios_fhir::r4::Bundle;
///
/// # #[cfg(feature = "R4")]
/// # {
/// // Parse from JSON
/// let json = r#"{
///     "resourceType": "Bundle",
///     "type": "collection",
///     "entry": [{
///         "resource": {
///             "resourceType": "Patient",
///             "id": "example"
///         }
///     }]
/// }"#;
///
/// let bundle: Bundle = serde_json::from_str(json)?;
/// let sof_bundle = SofBundle::R4(bundle);
///
/// // Check version compatibility
/// assert_eq!(sof_bundle.version(), helios_fhir::FhirVersion::R4);
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub enum SofBundle {
    #[cfg(feature = "R4")]
    R4(helios_fhir::r4::Bundle),
    #[cfg(feature = "R4B")]
    R4B(helios_fhir::r4b::Bundle),
    #[cfg(feature = "R5")]
    R5(helios_fhir::r5::Bundle),
    #[cfg(feature = "R6")]
    R6(helios_fhir::r6::Bundle),
}

impl SofBundle {
    /// Returns the FHIR specification version of this Bundle.
    ///
    /// This method provides version detection for multi-version applications,
    /// ensuring that ViewDefinitions and Bundles use compatible FHIR versions.
    ///
    /// # Returns
    ///
    /// The `FhirVersion` enum variant corresponding to this Bundle's specification.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_sof::SofBundle;
    /// use helios_fhir::FhirVersion;
    ///
    /// # #[cfg(feature = "R4")]
    /// # {
    /// # let bundle = helios_fhir::r4::Bundle::default();
    /// let sof_bundle = SofBundle::R4(bundle);
    /// assert_eq!(sof_bundle.version(), helios_fhir::FhirVersion::R4);
    /// # }
    /// ```
    pub fn version(&self) -> helios_fhir::FhirVersion {
        match self {
            #[cfg(feature = "R4")]
            SofBundle::R4(_) => helios_fhir::FhirVersion::R4,
            #[cfg(feature = "R4B")]
            SofBundle::R4B(_) => helios_fhir::FhirVersion::R4B,
            #[cfg(feature = "R5")]
            SofBundle::R5(_) => helios_fhir::FhirVersion::R5,
            #[cfg(feature = "R6")]
            SofBundle::R6(_) => helios_fhir::FhirVersion::R6,
        }
    }
}

/// Multi-version CapabilityStatement container supporting version-agnostic operations.
///
/// This enum provides a unified interface for working with CapabilityStatement resources
/// across different FHIR specification versions. It enables applications to handle
/// multiple FHIR versions simultaneously while maintaining type safety.
///
/// # Supported Versions
///
/// - **R4**: FHIR 4.0.1 CapabilityStatement (normative)
/// - **R4B**: FHIR 4.3.0 CapabilityStatement (ballot)
/// - **R5**: FHIR 5.0.0 CapabilityStatement (ballot)
/// - **R6**: FHIR 6.0.0 CapabilityStatement (draft)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SofCapabilityStatement {
    #[cfg(feature = "R4")]
    R4(helios_fhir::r4::CapabilityStatement),
    #[cfg(feature = "R4B")]
    R4B(helios_fhir::r4b::CapabilityStatement),
    #[cfg(feature = "R5")]
    R5(helios_fhir::r5::CapabilityStatement),
    #[cfg(feature = "R6")]
    R6(helios_fhir::r6::CapabilityStatement),
}

impl SofCapabilityStatement {
    /// Returns the FHIR specification version of this CapabilityStatement.
    pub fn version(&self) -> helios_fhir::FhirVersion {
        match self {
            #[cfg(feature = "R4")]
            SofCapabilityStatement::R4(_) => helios_fhir::FhirVersion::R4,
            #[cfg(feature = "R4B")]
            SofCapabilityStatement::R4B(_) => helios_fhir::FhirVersion::R4B,
            #[cfg(feature = "R5")]
            SofCapabilityStatement::R5(_) => helios_fhir::FhirVersion::R5,
            #[cfg(feature = "R6")]
            SofCapabilityStatement::R6(_) => helios_fhir::FhirVersion::R6,
        }
    }
}

/// Type alias for the version-independent Parameters container.
///
/// This alias provides backward compatibility while using the unified
/// VersionIndependentParameters from the helios_fhir crate.
pub type SofParameters = helios_fhir::VersionIndependentParameters;

/// Comprehensive error type for SQL-on-FHIR operations.
///
/// This enum covers all possible error conditions that can occur during
/// ViewDefinition processing, from validation failures to output formatting issues.
/// Each variant provides specific context about the error to aid in debugging.
///
/// # Error Categories
///
/// - **Validation**: ViewDefinition structure and logic validation
/// - **Evaluation**: FHIRPath expression evaluation failures
/// - **I/O**: File and serialization operations
/// - **Format**: Output format conversion issues
///
/// # Examples
///
/// ```rust,no_run
/// use helios_sof::{SofError, SofViewDefinition, SofBundle, ContentType, run_view_definition};
///
/// # let view = SofViewDefinition::R4(helios_fhir::r4::ViewDefinition::default());
/// # let bundle = SofBundle::R4(helios_fhir::r4::Bundle::default());
/// # let content_type = ContentType::Json;
/// match run_view_definition(view, bundle, content_type) {
///     Ok(output) => {
///         println!("Transformation successful");
///     },
///     Err(SofError::InvalidViewDefinition(msg)) => {
///         eprintln!("ViewDefinition validation failed: {}", msg);
///     },
///     Err(SofError::FhirPathError(msg)) => {
///         eprintln!("FHIRPath evaluation error: {}", msg);
///     },
///     Err(SofError::UnsupportedContentType(format)) => {
///         eprintln!("Unsupported output format: {}", format);
///     },
///     Err(e) => {
///         eprintln!("Other error: {}", e);
///     }
/// }
/// ```
#[derive(Debug, Error)]
pub enum SofError {
    /// ViewDefinition structure or logic validation failed.
    ///
    /// This error occurs when a ViewDefinition contains invalid or inconsistent
    /// configuration, such as missing required fields, invalid FHIRPath expressions,
    /// or incompatible select/unionAll structures.
    #[error("Invalid ViewDefinition: {0}")]
    InvalidViewDefinition(String),

    /// FHIRPath expression evaluation failed.
    ///
    /// This error occurs when a FHIRPath expression in a ViewDefinition cannot
    /// be evaluated, either due to syntax errors or runtime evaluation issues.
    #[error("FHIRPath evaluation error: {0}")]
    FhirPathError(String),

    /// JSON serialization/deserialization failed.
    ///
    /// This error occurs when parsing input JSON or serializing output data fails,
    /// typically due to malformed JSON or incompatible data structures.
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// CSV processing failed.
    ///
    /// This error occurs during CSV output generation, such as when writing
    /// headers or data rows to the CSV format.
    #[error("CSV error: {0}")]
    CsvError(#[from] csv::Error),

    /// File I/O operation failed.
    ///
    /// This error occurs when reading input files or writing output files fails,
    /// typically due to permission issues or missing files.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Unsupported output content type requested.
    ///
    /// This error occurs when an invalid or unimplemented content type is
    /// specified for output formatting.
    #[error("Unsupported content type: {0}")]
    UnsupportedContentType(String),

    /// CSV writer internal error.
    ///
    /// This error occurs when the CSV writer encounters an internal issue
    /// that prevents successful output generation.
    #[error("CSV writer error: {0}")]
    CsvWriterError(String),

    /// Invalid source parameter value.
    ///
    /// This error occurs when the source parameter contains an invalid URL or path.
    #[error("Invalid source: {0}")]
    InvalidSource(String),

    /// Source not found.
    ///
    /// This error occurs when the specified source file or URL cannot be found.
    #[error("Source not found: {0}")]
    SourceNotFound(String),

    /// Failed to fetch data from source.
    ///
    /// This error occurs when fetching data from a remote source fails.
    #[error("Failed to fetch source: {0}")]
    SourceFetchError(String),

    /// Failed to read source data.
    ///
    /// This error occurs when reading data from the source fails.
    #[error("Failed to read source: {0}")]
    SourceReadError(String),

    /// Invalid content in source.
    ///
    /// This error occurs when the source content is not valid FHIR data.
    #[error("Invalid source content: {0}")]
    InvalidSourceContent(String),

    /// Unsupported source protocol.
    ///
    /// This error occurs when the source URL uses an unsupported protocol.
    #[error("Unsupported source protocol: {0}")]
    UnsupportedSourceProtocol(String),

    /// Parquet conversion error.
    ///
    /// This error occurs when converting data to Parquet format fails.
    #[error("Parquet conversion error: {0}")]
    ParquetConversionError(String),
}

/// Supported output content types for ViewDefinition transformations.
///
/// This enum defines the available output formats for transformed FHIR data.
/// Each format has specific characteristics and use cases for different
/// integration scenarios.
///
/// # Format Descriptions
///
/// - **CSV**: Comma-separated values without headers
/// - **CSV with Headers**: Comma-separated values with column headers
/// - **JSON**: Pretty-printed JSON array of objects
/// - **NDJSON**: Newline-delimited JSON (one object per line)
/// - **Parquet**: Apache Parquet columnar format (planned)
///
/// # Examples
///
/// ```rust
/// use helios_sof::ContentType;
///
/// // Parse from string
/// let csv_type = ContentType::from_string("text/csv")?;
/// assert_eq!(csv_type, ContentType::CsvWithHeader);  // Default includes headers
///
/// let json_type = ContentType::from_string("application/json")?;
/// assert_eq!(json_type, ContentType::Json);
///
/// // CSV without headers
/// let csv_no_headers = ContentType::from_string("text/csv;header=false")?;
/// assert_eq!(csv_no_headers, ContentType::Csv);
/// # Ok::<(), helios_sof::SofError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    /// Comma-separated values format without headers
    Csv,
    /// Comma-separated values format with column headers
    CsvWithHeader,
    /// Pretty-printed JSON array format
    Json,
    /// Newline-delimited JSON format (NDJSON)
    NdJson,
    /// Apache Parquet columnar format (not yet implemented)
    Parquet,
}

impl ContentType {
    /// Parse a content type from its MIME type string representation.
    ///
    /// This method converts standard MIME type strings to the corresponding
    /// ContentType enum variants. It supports the SQL-on-FHIR specification's
    /// recommended content types.
    ///
    /// # Supported MIME Types
    ///
    /// - `"text/csv"` → [`ContentType::Csv`]
    /// - `"text/csv"` → [`ContentType::CsvWithHeader`] (default: headers included)
    /// - `"text/csv;header=true"` → [`ContentType::CsvWithHeader`]
    /// - `"text/csv;header=false"` → [`ContentType::Csv`]
    /// - `"application/json"` → [`ContentType::Json`]
    /// - `"application/ndjson"` → [`ContentType::NdJson`]
    /// - `"application/parquet"` → [`ContentType::Parquet`]
    ///
    /// # Arguments
    ///
    /// * `s` - The MIME type string to parse
    ///
    /// # Returns
    ///
    /// * `Ok(ContentType)` - Successfully parsed content type
    /// * `Err(SofError::UnsupportedContentType)` - Unknown or unsupported MIME type
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_sof::ContentType;
    ///
    /// // Shortened format names
    /// let csv = ContentType::from_string("csv")?;
    /// assert_eq!(csv, ContentType::CsvWithHeader);
    ///
    /// let json = ContentType::from_string("json")?;
    /// assert_eq!(json, ContentType::Json);
    ///
    /// let ndjson = ContentType::from_string("ndjson")?;
    /// assert_eq!(ndjson, ContentType::NdJson);
    ///
    /// // Full MIME types still supported
    /// let csv_mime = ContentType::from_string("text/csv")?;
    /// assert_eq!(csv_mime, ContentType::CsvWithHeader);
    ///
    /// // CSV with headers explicitly
    /// let csv_headers = ContentType::from_string("text/csv;header=true")?;
    /// assert_eq!(csv_headers, ContentType::CsvWithHeader);
    ///
    /// // CSV without headers
    /// let csv_no_headers = ContentType::from_string("text/csv;header=false")?;
    /// assert_eq!(csv_no_headers, ContentType::Csv);
    ///
    /// // JSON format
    /// let json_mime = ContentType::from_string("application/json")?;
    /// assert_eq!(json_mime, ContentType::Json);
    ///
    /// // Error for unsupported type
    /// assert!(ContentType::from_string("text/plain").is_err());
    /// # Ok::<(), helios_sof::SofError>(())
    /// ```
    pub fn from_string(s: &str) -> Result<Self, SofError> {
        match s {
            // Shortened format names
            "csv" => Ok(ContentType::CsvWithHeader),
            "json" => Ok(ContentType::Json),
            "ndjson" => Ok(ContentType::NdJson),
            "parquet" => Ok(ContentType::Parquet),
            // Full MIME types (for Accept header compatibility)
            "text/csv;header=false" => Ok(ContentType::Csv),
            "text/csv" | "text/csv;header=true" => Ok(ContentType::CsvWithHeader),
            "application/json" => Ok(ContentType::Json),
            "application/ndjson" => Ok(ContentType::NdJson),
            "application/parquet" => Ok(ContentType::Parquet),
            _ => Err(SofError::UnsupportedContentType(s.to_string())),
        }
    }
}

/// Returns the FHIR version string for the newest enabled version.
///
/// This function provides the version string that should be used in CapabilityStatements
/// and other FHIR resources that need to specify their version.
pub fn get_fhir_version_string() -> &'static str {
    let newest_version = get_newest_enabled_fhir_version();

    match newest_version {
        #[cfg(feature = "R4")]
        helios_fhir::FhirVersion::R4 => "4.0.1",
        #[cfg(feature = "R4B")]
        helios_fhir::FhirVersion::R4B => "4.3.0",
        #[cfg(feature = "R5")]
        helios_fhir::FhirVersion::R5 => "5.0.0",
        #[cfg(feature = "R6")]
        helios_fhir::FhirVersion::R6 => "6.0.0",
    }
}

/// Returns the newest FHIR version that is enabled at compile time.
///
/// This function uses compile-time feature detection to determine which FHIR
/// version should be used when multiple versions are enabled. The priority order
/// is: R6 > R5 > R4B > R4, where newer versions take precedence.
///
/// # Examples
///
/// ```rust
/// use helios_sof::{get_newest_enabled_fhir_version, FhirVersion};
///
/// # #[cfg(any(feature = "R4", feature = "R4B", feature = "R5", feature = "R6"))]
/// # {
/// let version = get_newest_enabled_fhir_version();
/// // If R5 and R4 are both enabled, this returns R5
/// # }
/// ```
///
/// # Panics
///
/// This function will panic at compile time if no FHIR version features are enabled.
pub fn get_newest_enabled_fhir_version() -> helios_fhir::FhirVersion {
    #[cfg(feature = "R6")]
    return helios_fhir::FhirVersion::R6;

    #[cfg(all(feature = "R5", not(feature = "R6")))]
    return helios_fhir::FhirVersion::R5;

    #[cfg(all(feature = "R4B", not(feature = "R5"), not(feature = "R6")))]
    return helios_fhir::FhirVersion::R4B;

    #[cfg(all(
        feature = "R4",
        not(feature = "R4B"),
        not(feature = "R5"),
        not(feature = "R6")
    ))]
    return helios_fhir::FhirVersion::R4;

    #[cfg(not(any(feature = "R4", feature = "R4B", feature = "R5", feature = "R6")))]
    panic!("At least one FHIR version feature must be enabled");
}

/// A single row of processed tabular data from ViewDefinition transformation.
///
/// This struct represents one row in the output table, containing values for
/// each column defined in the ViewDefinition. Values are stored as optional
/// JSON values to handle nullable fields and diverse FHIR data types.
///
/// # Structure
///
/// Each `ProcessedRow` contains a vector of optional JSON values, where:
/// - `Some(value)` represents a non-null column value
/// - `None` represents a null/missing column value
/// - The order matches the column order in [`ProcessedResult::columns`]
///
/// # Examples
///
/// ```rust
/// use helios_sof::ProcessedRow;
/// use serde_json::Value;
///
/// let row = ProcessedRow {
///     values: vec![
///         Some(Value::String("patient-123".to_string())),
///         Some(Value::String("Doe".to_string())),
///         None, // Missing birth date
///         Some(Value::Bool(true)),
///     ]
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedRow {
    /// Column values for this row, ordered according to ProcessedResult::columns
    pub values: Vec<Option<serde_json::Value>>,
}

/// Complete result of ViewDefinition transformation containing columns and data rows.
///
/// This struct represents the tabular output from processing a ViewDefinition
/// against a Bundle of FHIR resources. It contains both the column definitions
/// and the actual data rows in a format ready for serialization to various
/// output formats.
///
/// # Structure
///
/// - [`columns`](Self::columns): Ordered list of column names from the ViewDefinition
/// - [`rows`](Self::rows): Data rows where each row contains values in column order
///
/// # Examples
///
/// ```rust
/// use helios_sof::{ProcessedResult, ProcessedRow};
/// use serde_json::Value;
///
/// let result = ProcessedResult {
///     columns: vec![
///         "patient_id".to_string(),
///         "family_name".to_string(),
///         "given_name".to_string(),
///     ],
///     rows: vec![
///         ProcessedRow {
///             values: vec![
///                 Some(Value::String("patient-1".to_string())),
///                 Some(Value::String("Smith".to_string())),
///                 Some(Value::String("John".to_string())),
///             ]
///         },
///         ProcessedRow {
///             values: vec![
///                 Some(Value::String("patient-2".to_string())),
///                 Some(Value::String("Doe".to_string())),
///                 None, // Missing given name
///             ]
///         },
///     ]
/// };
///
/// assert_eq!(result.columns.len(), 3);
/// assert_eq!(result.rows.len(), 2);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedResult {
    /// Ordered list of column names as defined in the ViewDefinition
    pub columns: Vec<String>,
    /// Data rows containing values for each column
    pub rows: Vec<ProcessedRow>,
}

/// Execute a SQL-on-FHIR ViewDefinition transformation on a FHIR Bundle.
///
/// This is the main entry point for SQL-on-FHIR transformations. It processes
/// a ViewDefinition against a Bundle of FHIR resources and produces output in
/// the specified format. The function handles version compatibility, validation,
/// FHIRPath evaluation, and output formatting.
///
/// # Arguments
///
/// * `view_definition` - The ViewDefinition containing transformation logic
/// * `bundle` - The Bundle containing FHIR resources to process
/// * `content_type` - The desired output format
///
/// # Returns
///
/// * `Ok(Vec<u8>)` - Formatted output bytes ready for writing to file or stdout
/// * `Err(SofError)` - Detailed error information about what went wrong
///
/// # Validation
///
/// The function performs comprehensive validation:
/// - FHIR version compatibility between ViewDefinition and Bundle
/// - ViewDefinition structure and logic validation
/// - FHIRPath expression syntax and evaluation
/// - Output format compatibility
///
/// # Examples
///
/// ```rust
/// use helios_sof::{SofViewDefinition, SofBundle, ContentType, run_view_definition};
///
/// # #[cfg(feature = "R4")]
/// # {
/// // Create a simple ViewDefinition
/// let view_json = serde_json::json!({
///     "resourceType": "ViewDefinition",
///     "status": "active",
///     "resource": "Patient",
///     "select": [{
///         "column": [{
///             "name": "id",
///             "path": "id"
///         }]
///     }]
/// });
/// let view_def: helios_fhir::r4::ViewDefinition = serde_json::from_value(view_json)?;
///
/// // Create a simple Bundle
/// let bundle_json = serde_json::json!({
///     "resourceType": "Bundle",
///     "type": "collection",
///     "entry": []
/// });
/// let bundle: helios_fhir::r4::Bundle = serde_json::from_value(bundle_json)?;
///
/// let sof_view = SofViewDefinition::R4(view_def);
/// let sof_bundle = SofBundle::R4(bundle);
///
/// // Generate CSV with headers
/// let csv_output = run_view_definition(
///     sof_view,
///     sof_bundle,
///     ContentType::CsvWithHeader
/// )?;
///
/// // Write to file or stdout
/// std::fs::write("output.csv", csv_output)?;
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Error Handling
///
/// Common error scenarios:
///
/// ```rust,no_run
/// use helios_sof::{SofError, SofViewDefinition, SofBundle, ContentType, run_view_definition};
///
/// # let view = SofViewDefinition::R4(helios_fhir::r4::ViewDefinition::default());
/// # let bundle = SofBundle::R4(helios_fhir::r4::Bundle::default());
/// # let content_type = ContentType::Json;
/// match run_view_definition(view, bundle, content_type) {
///     Ok(output) => {
///         println!("Success: {} bytes generated", output.len());
///     },
///     Err(SofError::InvalidViewDefinition(msg)) => {
///         eprintln!("ViewDefinition error: {}", msg);
///     },
///     Err(SofError::FhirPathError(msg)) => {
///         eprintln!("FHIRPath error: {}", msg);
///     },
///     Err(e) => {
///         eprintln!("Other error: {}", e);
///     }
/// }
/// ```
pub fn run_view_definition(
    view_definition: SofViewDefinition,
    bundle: SofBundle,
    content_type: ContentType,
) -> Result<Vec<u8>, SofError> {
    run_view_definition_with_options(view_definition, bundle, content_type, RunOptions::default())
}

/// Configuration options for Parquet file generation.
#[derive(Debug, Clone)]
pub struct ParquetOptions {
    /// Target row group size in MB (64-1024)
    pub row_group_size_mb: u32,
    /// Target page size in KB (64-8192)
    pub page_size_kb: u32,
    /// Compression algorithm (none, snappy, gzip, lz4, brotli, zstd)
    pub compression: String,
    /// Maximum file size in MB (splits output when exceeded)
    pub max_file_size_mb: Option<u32>,
}

impl Default for ParquetOptions {
    fn default() -> Self {
        Self {
            row_group_size_mb: 256,
            page_size_kb: 1024,
            compression: "snappy".to_string(),
            max_file_size_mb: None,
        }
    }
}

/// Options for filtering and controlling ViewDefinition execution
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Filter resources modified after this time
    pub since: Option<DateTime<Utc>>,
    /// Limit the number of results
    pub limit: Option<usize>,
    /// Page number for pagination (1-based)
    pub page: Option<usize>,
    /// Parquet-specific configuration options
    pub parquet_options: Option<ParquetOptions>,
}

/// Execute a ViewDefinition transformation with additional filtering options.
///
/// This function extends the basic `run_view_definition` with support for:
/// - Filtering resources by modification time (`since`)
/// - Limiting results (`limit`)
/// - Pagination (`page`)
///
/// # Arguments
///
/// * `view_definition` - The ViewDefinition to execute
/// * `bundle` - The Bundle containing resources to transform
/// * `content_type` - Desired output format
/// * `options` - Additional filtering and control options
///
/// # Returns
///
/// The transformed data in the requested format, with filtering applied.
pub fn run_view_definition_with_options(
    view_definition: SofViewDefinition,
    bundle: SofBundle,
    content_type: ContentType,
    options: RunOptions,
) -> Result<Vec<u8>, SofError> {
    // Filter bundle resources by since parameter before processing
    let filtered_bundle = if let Some(since) = options.since {
        filter_bundle_by_since(bundle, since)?
    } else {
        bundle
    };

    // Process the ViewDefinition to generate tabular data
    let processed_result = process_view_definition(view_definition, filtered_bundle)?;

    // Apply pagination if needed
    let processed_result = if options.limit.is_some() || options.page.is_some() {
        apply_pagination_to_result(processed_result, options.limit, options.page)?
    } else {
        processed_result
    };

    // Format the result according to the requested content type
    format_output(
        processed_result,
        content_type,
        options.parquet_options.as_ref(),
    )
}

pub fn process_view_definition(
    view_definition: SofViewDefinition,
    bundle: SofBundle,
) -> Result<ProcessedResult, SofError> {
    // Ensure both resources use the same FHIR version
    if view_definition.version() != bundle.version() {
        return Err(SofError::InvalidViewDefinition(
            "ViewDefinition and Bundle must use the same FHIR version".to_string(),
        ));
    }

    match (view_definition, bundle) {
        #[cfg(feature = "R4")]
        (SofViewDefinition::R4(vd), SofBundle::R4(bundle)) => {
            process_view_definition_generic(vd, bundle)
        }
        #[cfg(feature = "R4B")]
        (SofViewDefinition::R4B(vd), SofBundle::R4B(bundle)) => {
            process_view_definition_generic(vd, bundle)
        }
        #[cfg(feature = "R5")]
        (SofViewDefinition::R5(vd), SofBundle::R5(bundle)) => {
            process_view_definition_generic(vd, bundle)
        }
        #[cfg(feature = "R6")]
        (SofViewDefinition::R6(vd), SofBundle::R6(bundle)) => {
            process_view_definition_generic(vd, bundle)
        }
        // This case should never happen due to the version check above,
        // but is needed for exhaustive pattern matching when multiple features are enabled
        #[cfg(any(
            all(feature = "R4", any(feature = "R4B", feature = "R5", feature = "R6")),
            all(feature = "R4B", any(feature = "R5", feature = "R6")),
            all(feature = "R5", feature = "R6")
        ))]
        _ => {
            unreachable!("Version mismatch should have been caught by the version check above")
        }
    }
}

// Generic version-agnostic constant extraction
fn extract_view_definition_constants<VD: ViewDefinitionTrait>(
    view_definition: &VD,
) -> Result<HashMap<String, EvaluationResult>, SofError> {
    let mut variables = HashMap::new();

    if let Some(constants) = view_definition.constants() {
        for constant in constants {
            let name = constant
                .name()
                .ok_or_else(|| {
                    SofError::InvalidViewDefinition("Constant name is required".to_string())
                })?
                .to_string();

            let eval_result = constant.to_evaluation_result()?;
            // Constants are referenced with % prefix in FHIRPath expressions
            variables.insert(format!("%{}", name), eval_result);
        }
    }

    Ok(variables)
}

// Generic version-agnostic ViewDefinition processing
fn process_view_definition_generic<VD, B>(
    view_definition: VD,
    bundle: B,
) -> Result<ProcessedResult, SofError>
where
    VD: ViewDefinitionTrait,
    B: BundleTrait,
    B::Resource: ResourceTrait + Sync,
    VD::Select: Sync,
{
    validate_view_definition(&view_definition)?;

    // Step 1: Extract constants/variables from ViewDefinition
    let variables = extract_view_definition_constants(&view_definition)?;

    // Step 2: Filter resources by type and profile
    let target_resource_type = view_definition
        .resource()
        .ok_or_else(|| SofError::InvalidViewDefinition("Resource type is required".to_string()))?;

    let filtered_resources = filter_resources(&bundle, target_resource_type)?;

    // Step 3: Apply where clauses to filter resources
    let filtered_resources = apply_where_clauses(
        filtered_resources,
        view_definition.where_clauses(),
        &variables,
    )?;

    // Step 4: Process all select clauses to generate rows with forEach support
    let select_clauses = view_definition.select().ok_or_else(|| {
        SofError::InvalidViewDefinition("At least one select clause is required".to_string())
    })?;

    // Generate rows for each resource using the forEach-aware approach
    let (all_columns, rows) =
        generate_rows_from_selects(&filtered_resources, select_clauses, &variables)?;

    Ok(ProcessedResult {
        columns: all_columns,
        rows,
    })
}

// Generic version-agnostic validation
fn validate_view_definition<VD: ViewDefinitionTrait>(view_def: &VD) -> Result<(), SofError> {
    // Basic validation
    if view_def.resource().is_none_or(|s| s.is_empty()) {
        return Err(SofError::InvalidViewDefinition(
            "ViewDefinition must specify a resource type".to_string(),
        ));
    }

    if view_def.select().is_none_or(|s| s.is_empty()) {
        return Err(SofError::InvalidViewDefinition(
            "ViewDefinition must have at least one select".to_string(),
        ));
    }

    // Validate where clauses
    if let Some(where_clauses) = view_def.where_clauses() {
        validate_where_clauses(where_clauses)?;
    }

    // Validate selects
    if let Some(selects) = view_def.select() {
        for select in selects {
            validate_select(select)?;
        }
    }

    Ok(())
}

// Generic where clause validation
fn validate_where_clauses<W: ViewDefinitionWhereTrait>(
    where_clauses: &[W],
) -> Result<(), SofError> {
    // Basic validation - just ensure paths are provided
    // Type checking will be done during actual evaluation
    for where_clause in where_clauses {
        if where_clause.path().is_none() {
            return Err(SofError::InvalidViewDefinition(
                "Where clause must have a path specified".to_string(),
            ));
        }
    }
    Ok(())
}

// Generic helper - no longer needs to be version-specific
fn can_be_coerced_to_boolean(result: &EvaluationResult) -> bool {
    // Check if the result can be meaningfully used as a boolean in a where clause
    match result {
        // Boolean values are obviously OK
        EvaluationResult::Boolean(_, _) => true,

        // Empty is OK (evaluates to false)
        EvaluationResult::Empty => true,

        // Collections are OK - they evaluate based on whether they're empty or not
        EvaluationResult::Collection { .. } => true,

        // Other types cannot be meaningfully coerced to boolean for where clauses
        // This includes: String, Integer, Decimal, Date, DateTime, Time, Quantity, Object
        _ => false,
    }
}

// Generic select validation
fn validate_select<S: ViewDefinitionSelectTrait>(select: &S) -> Result<(), SofError> {
    validate_select_with_context(select, false)
}

fn validate_select_with_context<S: ViewDefinitionSelectTrait>(
    select: &S,
    in_foreach_context: bool,
) -> Result<(), SofError>
where
    S::Select: ViewDefinitionSelectTrait,
{
    // Determine if we're entering a forEach context at this level
    let entering_foreach = select.for_each().is_some() || select.for_each_or_null().is_some();
    let current_foreach_context = in_foreach_context || entering_foreach;

    // Validate collection attribute with the current forEach context
    if let Some(columns) = select.column() {
        for column in columns {
            if let Some(collection_value) = column.collection() {
                if !collection_value && !current_foreach_context {
                    return Err(SofError::InvalidViewDefinition(
                        "Column 'collection' attribute must be true when specified".to_string(),
                    ));
                }
            }
        }
    }

    // Validate unionAll column consistency
    if let Some(union_selects) = select.union_all() {
        validate_union_all_columns(union_selects)?;
    }

    // Recursively validate nested selects
    if let Some(nested_selects) = select.select() {
        for nested_select in nested_selects {
            validate_select_with_context(nested_select, current_foreach_context)?;
        }
    }

    // Validate unionAll selects with forEach context
    if let Some(union_selects) = select.union_all() {
        for union_select in union_selects {
            validate_select_with_context(union_select, current_foreach_context)?;
        }
    }

    Ok(())
}

// Generic union validation
fn validate_union_all_columns<S: ViewDefinitionSelectTrait>(
    union_selects: &[S],
) -> Result<(), SofError> {
    if union_selects.len() < 2 {
        return Ok(());
    }

    // Get column names and order from first select
    let first_select = &union_selects[0];
    let first_columns = get_column_names(first_select)?;

    // Validate all other selects have the same column names in the same order
    for (index, union_select) in union_selects.iter().enumerate().skip(1) {
        let current_columns = get_column_names(union_select)?;

        if current_columns != first_columns {
            if current_columns.len() != first_columns.len()
                || !current_columns
                    .iter()
                    .all(|name| first_columns.contains(name))
            {
                return Err(SofError::InvalidViewDefinition(format!(
                    "UnionAll branch {} has different column names than first branch",
                    index
                )));
            } else {
                return Err(SofError::InvalidViewDefinition(format!(
                    "UnionAll branch {} has columns in different order than first branch",
                    index
                )));
            }
        }
    }

    Ok(())
}

// Generic column name extraction
fn get_column_names<S: ViewDefinitionSelectTrait>(select: &S) -> Result<Vec<String>, SofError> {
    let mut column_names = Vec::new();

    // Collect direct column names
    if let Some(columns) = select.column() {
        for column in columns {
            if let Some(name) = column.name() {
                column_names.push(name.to_string());
            }
        }
    }

    // If this select has unionAll but no direct columns, get columns from first unionAll branch
    if column_names.is_empty() {
        if let Some(union_selects) = select.union_all() {
            if !union_selects.is_empty() {
                return get_column_names(&union_selects[0]);
            }
        }
    }

    Ok(column_names)
}

// Generic resource filtering
fn filter_resources<'a, B: BundleTrait>(
    bundle: &'a B,
    resource_type: &str,
) -> Result<Vec<&'a B::Resource>, SofError> {
    Ok(bundle
        .entries()
        .into_iter()
        .filter(|resource| resource.resource_name() == resource_type)
        .collect())
}

// Generic where clause application
fn apply_where_clauses<'a, R, W>(
    resources: Vec<&'a R>,
    where_clauses: Option<&[W]>,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<&'a R>, SofError>
where
    R: ResourceTrait,
    W: ViewDefinitionWhereTrait,
{
    if let Some(wheres) = where_clauses {
        let mut filtered = Vec::new();

        for resource in resources {
            let mut include_resource = true;

            // All where clauses must evaluate to true for the resource to be included
            for where_clause in wheres {
                let fhir_resource = resource.to_fhir_resource();
                let mut context = EvaluationContext::new(vec![fhir_resource]);

                // Add variables to the context
                for (name, value) in variables {
                    context.set_variable_result(name, value.clone());
                }

                let path = where_clause.path().ok_or_else(|| {
                    SofError::InvalidViewDefinition("Where clause path is required".to_string())
                })?;

                match evaluate_expression(path, &context) {
                    Ok(result) => {
                        // Check if the result can be meaningfully used as a boolean
                        if !can_be_coerced_to_boolean(&result) {
                            return Err(SofError::InvalidViewDefinition(format!(
                                "Where clause path '{}' returns type '{}' which cannot be used as a boolean condition. \
                                 Where clauses must return boolean values, collections, or empty results.",
                                path,
                                result.type_name()
                            )));
                        }

                        // Check if result is truthy (non-empty and not false)
                        if !is_truthy(&result) {
                            include_resource = false;
                            break;
                        }
                    }
                    Err(e) => {
                        return Err(SofError::FhirPathError(format!(
                            "Error evaluating where clause '{}': {}",
                            path, e
                        )));
                    }
                }
            }

            if include_resource {
                filtered.push(resource);
            }
        }

        Ok(filtered)
    } else {
        Ok(resources)
    }
}

// Removed generate_rows_per_resource_r4 - replaced with new forEach-aware implementation

// Removed generate_rows_with_for_each_r4 - replaced with new forEach-aware implementation

// Helper functions for FHIRPath result processing
fn is_truthy(result: &EvaluationResult) -> bool {
    match result {
        EvaluationResult::Empty => false,
        EvaluationResult::Boolean(b, _) => *b,
        EvaluationResult::Collection { items, .. } => !items.is_empty(),
        _ => true, // Non-empty, non-false values are truthy
    }
}

fn fhirpath_result_to_json_value_collection(result: EvaluationResult) -> Option<serde_json::Value> {
    match result {
        EvaluationResult::Empty => Some(serde_json::Value::Array(vec![])),
        EvaluationResult::Collection { items, .. } => {
            // Always return array for collection columns, even if empty
            let values: Vec<serde_json::Value> = items
                .into_iter()
                .filter_map(fhirpath_result_to_json_value)
                .collect();
            Some(serde_json::Value::Array(values))
        }
        // For non-collection results in collection columns, wrap in array
        single_result => {
            if let Some(json_val) = fhirpath_result_to_json_value(single_result) {
                Some(serde_json::Value::Array(vec![json_val]))
            } else {
                Some(serde_json::Value::Array(vec![]))
            }
        }
    }
}

fn fhirpath_result_to_json_value(result: EvaluationResult) -> Option<serde_json::Value> {
    match result {
        EvaluationResult::Empty => None,
        EvaluationResult::Boolean(b, _) => Some(serde_json::Value::Bool(b)),
        EvaluationResult::Integer(i, _) => {
            Some(serde_json::Value::Number(serde_json::Number::from(i)))
        }
        EvaluationResult::Decimal(d, _) => {
            // Check if this Decimal represents a whole number
            if d.fract().is_zero() {
                // Convert to integer if no fractional part
                if let Ok(i) = d.to_string().parse::<i64>() {
                    Some(serde_json::Value::Number(serde_json::Number::from(i)))
                } else {
                    // Handle very large numbers as strings
                    Some(serde_json::Value::String(d.to_string()))
                }
            } else {
                // Convert Decimal to a float for fractional numbers
                if let Ok(f) = d.to_string().parse::<f64>() {
                    if let Some(num) = serde_json::Number::from_f64(f) {
                        Some(serde_json::Value::Number(num))
                    } else {
                        Some(serde_json::Value::String(d.to_string()))
                    }
                } else {
                    Some(serde_json::Value::String(d.to_string()))
                }
            }
        }
        EvaluationResult::String(s, _) => Some(serde_json::Value::String(s)),
        EvaluationResult::Date(s, _) => Some(serde_json::Value::String(s)),
        EvaluationResult::DateTime(s, _) => {
            // Remove "@" prefix from datetime strings if present
            let cleaned = s.strip_prefix("@").unwrap_or(&s);
            Some(serde_json::Value::String(cleaned.to_string()))
        }
        EvaluationResult::Time(s, _) => {
            // Remove "@T" prefix from time strings if present
            let cleaned = s.strip_prefix("@T").unwrap_or(&s);
            Some(serde_json::Value::String(cleaned.to_string()))
        }
        EvaluationResult::Collection { items, .. } => {
            if items.len() == 1 {
                // Single item collection - unwrap to the item itself
                fhirpath_result_to_json_value(items.into_iter().next().unwrap())
            } else if items.is_empty() {
                None
            } else {
                // Multiple items - convert to array
                let values: Vec<serde_json::Value> = items
                    .into_iter()
                    .filter_map(fhirpath_result_to_json_value)
                    .collect();
                Some(serde_json::Value::Array(values))
            }
        }
        EvaluationResult::Object { map, .. } => {
            let mut json_map = serde_json::Map::new();
            for (k, v) in map {
                if let Some(json_val) = fhirpath_result_to_json_value(v) {
                    json_map.insert(k, json_val);
                }
            }
            Some(serde_json::Value::Object(json_map))
        }
        // Handle other result types as strings
        _ => Some(serde_json::Value::String(format!("{:?}", result))),
    }
}

fn extract_iteration_items(result: EvaluationResult) -> Vec<EvaluationResult> {
    match result {
        EvaluationResult::Collection { items, .. } => items,
        EvaluationResult::Empty => Vec::new(),
        single_item => vec![single_item],
    }
}

// Generic row generation functions

fn generate_rows_from_selects<R, S>(
    resources: &[&R],
    selects: &[S],
    variables: &HashMap<String, EvaluationResult>,
) -> Result<(Vec<String>, Vec<ProcessedRow>), SofError>
where
    R: ResourceTrait + Sync,
    S: ViewDefinitionSelectTrait + Sync,
    S::Select: ViewDefinitionSelectTrait,
{
    // Process resources in parallel
    let resource_results: Result<Vec<_>, _> = resources
        .par_iter()
        .map(|resource| {
            // Each thread gets its own local column vector
            let mut local_columns = Vec::new();
            let resource_rows =
                generate_rows_for_resource(*resource, selects, &mut local_columns, variables)?;
            Ok::<(Vec<String>, Vec<ProcessedRow>), SofError>((local_columns, resource_rows))
        })
        .collect();

    // Handle errors from parallel processing
    let resource_results = resource_results?;

    // Merge columns from all threads (maintaining order is important)
    let mut final_columns = Vec::new();
    let mut all_rows = Vec::new();

    for (local_columns, resource_rows) in resource_results {
        // Merge columns, avoiding duplicates
        for col in local_columns {
            if !final_columns.contains(&col) {
                final_columns.push(col);
            }
        }
        all_rows.extend(resource_rows);
    }

    Ok((final_columns, all_rows))
}

fn generate_rows_for_resource<R, S>(
    resource: &R,
    selects: &[S],
    all_columns: &mut Vec<String>,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<ProcessedRow>, SofError>
where
    R: ResourceTrait,
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    let fhir_resource = resource.to_fhir_resource();
    let mut context = EvaluationContext::new(vec![fhir_resource]);

    // Add variables to the context
    for (name, value) in variables {
        context.set_variable_result(name, value.clone());
    }

    // Generate all possible row combinations for this resource
    let row_combinations = generate_row_combinations(&context, selects, all_columns, variables)?;

    Ok(row_combinations)
}

#[derive(Debug, Clone)]
struct RowCombination {
    values: Vec<Option<serde_json::Value>>,
}

fn generate_row_combinations<S>(
    context: &EvaluationContext,
    selects: &[S],
    all_columns: &mut Vec<String>,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<ProcessedRow>, SofError>
where
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    // First pass: collect all column names to ensure consistent ordering
    collect_all_columns(selects, all_columns)?;

    // Second pass: generate all row combinations
    let mut row_combinations = vec![RowCombination {
        values: vec![None; all_columns.len()],
    }];

    for select in selects {
        row_combinations =
            expand_select_combinations(context, select, &row_combinations, all_columns, variables)?;
    }

    // Convert to ProcessedRow format
    Ok(row_combinations
        .into_iter()
        .map(|combo| ProcessedRow {
            values: combo.values,
        })
        .collect())
}

fn collect_all_columns<S>(selects: &[S], all_columns: &mut Vec<String>) -> Result<(), SofError>
where
    S: ViewDefinitionSelectTrait,
{
    for select in selects {
        // Add columns from this select
        if let Some(columns) = select.column() {
            for col in columns {
                if let Some(name) = col.name() {
                    if !all_columns.contains(&name.to_string()) {
                        all_columns.push(name.to_string());
                    }
                }
            }
        }

        // Recursively collect from nested selects
        if let Some(nested_selects) = select.select() {
            collect_all_columns(nested_selects, all_columns)?;
        }

        // Collect from unionAll
        if let Some(union_selects) = select.union_all() {
            collect_all_columns(union_selects, all_columns)?;
        }
    }
    Ok(())
}

fn expand_select_combinations<S>(
    context: &EvaluationContext,
    select: &S,
    existing_combinations: &[RowCombination],
    all_columns: &[String],
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<RowCombination>, SofError>
where
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    // Handle forEach and forEachOrNull
    if let Some(for_each_path) = select.for_each() {
        return expand_for_each_combinations(
            context,
            select,
            existing_combinations,
            all_columns,
            for_each_path,
            false,
            variables,
        );
    }

    if let Some(for_each_or_null_path) = select.for_each_or_null() {
        return expand_for_each_combinations(
            context,
            select,
            existing_combinations,
            all_columns,
            for_each_or_null_path,
            true,
            variables,
        );
    }

    // Handle regular columns (no forEach)
    let mut new_combinations = Vec::new();

    for existing_combo in existing_combinations {
        let mut new_combo = existing_combo.clone();

        // Add values from this select's columns
        if let Some(columns) = select.column() {
            for col in columns {
                if let Some(col_name) = col.name() {
                    if let Some(col_index) = all_columns.iter().position(|name| name == col_name) {
                        let path = col.path().ok_or_else(|| {
                            SofError::InvalidViewDefinition("Column path is required".to_string())
                        })?;

                        match evaluate_expression(path, context) {
                            Ok(result) => {
                                // Check if this column is marked as a collection
                                let is_collection = col.collection().unwrap_or(false);

                                new_combo.values[col_index] = if is_collection {
                                    fhirpath_result_to_json_value_collection(result)
                                } else {
                                    fhirpath_result_to_json_value(result)
                                };
                            }
                            Err(e) => {
                                return Err(SofError::FhirPathError(format!(
                                    "Error evaluating column '{}' with path '{}': {}",
                                    col_name, path, e
                                )));
                            }
                        }
                    }
                }
            }
        }

        new_combinations.push(new_combo);
    }

    // Handle nested selects
    if let Some(nested_selects) = select.select() {
        for nested_select in nested_selects {
            new_combinations = expand_select_combinations(
                context,
                nested_select,
                &new_combinations,
                all_columns,
                variables,
            )?;
        }
    }

    // Handle unionAll
    if let Some(union_selects) = select.union_all() {
        let mut union_combinations = Vec::new();

        // Process each unionAll select independently, using the combinations that already have
        // values from this select's columns and nested selects
        for union_select in union_selects {
            let select_combinations = expand_select_combinations(
                context,
                union_select,
                &new_combinations,
                all_columns,
                variables,
            )?;
            union_combinations.extend(select_combinations);
        }

        // unionAll replaces new_combinations with the union results
        // If no union results, this resource should be filtered out (no rows for this resource)
        new_combinations = union_combinations;
    }

    Ok(new_combinations)
}

fn expand_for_each_combinations<S>(
    context: &EvaluationContext,
    select: &S,
    existing_combinations: &[RowCombination],
    all_columns: &[String],
    for_each_path: &str,
    allow_null: bool,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<RowCombination>, SofError>
where
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    // Evaluate the forEach expression to get iteration items
    let for_each_result = evaluate_expression(for_each_path, context).map_err(|e| {
        SofError::FhirPathError(format!(
            "Error evaluating forEach expression '{}': {}",
            for_each_path, e
        ))
    })?;

    let iteration_items = extract_iteration_items(for_each_result);

    if iteration_items.is_empty() {
        if allow_null {
            // forEachOrNull: generate null rows
            let mut new_combinations = Vec::new();
            for existing_combo in existing_combinations {
                let mut new_combo = existing_combo.clone();

                // Set column values to null for this forEach scope
                if let Some(columns) = select.column() {
                    for col in columns {
                        if let Some(col_name) = col.name() {
                            if let Some(col_index) =
                                all_columns.iter().position(|name| name == col_name)
                            {
                                new_combo.values[col_index] = None;
                            }
                        }
                    }
                }

                new_combinations.push(new_combo);
            }
            return Ok(new_combinations);
        } else {
            // forEach with empty collection: no rows
            return Ok(Vec::new());
        }
    }

    let mut new_combinations = Vec::new();

    // For each iteration item, create new combinations
    for item in &iteration_items {
        // Create a new context with the iteration item
        let _item_context = create_iteration_context(item, variables);

        for existing_combo in existing_combinations {
            let mut new_combo = existing_combo.clone();

            // Evaluate columns in the context of the iteration item
            if let Some(columns) = select.column() {
                for col in columns {
                    if let Some(col_name) = col.name() {
                        if let Some(col_index) =
                            all_columns.iter().position(|name| name == col_name)
                        {
                            let path = col.path().ok_or_else(|| {
                                SofError::InvalidViewDefinition(
                                    "Column path is required".to_string(),
                                )
                            })?;

                            // Use the iteration item directly for path evaluation
                            let result = if path == "$this" {
                                // Special case: $this refers to the current iteration item
                                item.clone()
                            } else {
                                // Evaluate the path on the iteration item
                                evaluate_path_on_item(path, item, variables)?
                            };

                            // Check if this column is marked as a collection
                            let is_collection = col.collection().unwrap_or(false);

                            new_combo.values[col_index] = if is_collection {
                                fhirpath_result_to_json_value_collection(result)
                            } else {
                                fhirpath_result_to_json_value(result)
                            };
                        }
                    }
                }
            }

            new_combinations.push(new_combo);
        }
    }

    // Handle nested selects with the forEach context
    if let Some(nested_selects) = select.select() {
        let mut final_combinations = Vec::new();

        for item in &iteration_items {
            let item_context = create_iteration_context(item, variables);

            // For each iteration item, we need to start with the combinations that have
            // the correct column values for this forEach scope
            for existing_combo in existing_combinations {
                // Find the combination that corresponds to this iteration item
                // by looking at the values we set for columns in this forEach scope
                let mut base_combo = existing_combo.clone();

                // Update the base combination with column values for this iteration item
                if let Some(columns) = select.column() {
                    for col in columns {
                        if let Some(col_name) = col.name() {
                            if let Some(col_index) =
                                all_columns.iter().position(|name| name == col_name)
                            {
                                let path = col.path().ok_or_else(|| {
                                    SofError::InvalidViewDefinition(
                                        "Column path is required".to_string(),
                                    )
                                })?;

                                let result = if path == "$this" {
                                    item.clone()
                                } else {
                                    evaluate_path_on_item(path, item, variables)?
                                };

                                // Check if this column is marked as a collection
                                let is_collection = col.collection().unwrap_or(false);

                                base_combo.values[col_index] = if is_collection {
                                    fhirpath_result_to_json_value_collection(result)
                                } else {
                                    fhirpath_result_to_json_value(result)
                                };
                            }
                        }
                    }
                }

                // Start with this base combination for nested processing
                let mut item_combinations = vec![base_combo];

                // Process nested selects
                for nested_select in nested_selects {
                    item_combinations = expand_select_combinations(
                        &item_context,
                        nested_select,
                        &item_combinations,
                        all_columns,
                        variables,
                    )?;
                }

                final_combinations.extend(item_combinations);
            }
        }

        new_combinations = final_combinations;
    }

    // Handle unionAll within forEach context
    if let Some(union_selects) = select.union_all() {
        let mut union_combinations = Vec::new();

        for item in &iteration_items {
            let item_context = create_iteration_context(item, variables);

            // For each iteration item, process all unionAll selects
            for existing_combo in existing_combinations {
                let mut base_combo = existing_combo.clone();

                // Update the base combination with column values for this iteration item
                if let Some(columns) = select.column() {
                    for col in columns {
                        if let Some(col_name) = col.name() {
                            if let Some(col_index) =
                                all_columns.iter().position(|name| name == col_name)
                            {
                                let path = col.path().ok_or_else(|| {
                                    SofError::InvalidViewDefinition(
                                        "Column path is required".to_string(),
                                    )
                                })?;

                                let result = if path == "$this" {
                                    item.clone()
                                } else {
                                    evaluate_path_on_item(path, item, variables)?
                                };

                                // Check if this column is marked as a collection
                                let is_collection = col.collection().unwrap_or(false);

                                base_combo.values[col_index] = if is_collection {
                                    fhirpath_result_to_json_value_collection(result)
                                } else {
                                    fhirpath_result_to_json_value(result)
                                };
                            }
                        }
                    }
                }

                // Also evaluate columns from nested selects and add them to base_combo
                if let Some(nested_selects) = select.select() {
                    for nested_select in nested_selects {
                        if let Some(nested_columns) = nested_select.column() {
                            for col in nested_columns {
                                if let Some(col_name) = col.name() {
                                    if let Some(col_index) =
                                        all_columns.iter().position(|name| name == col_name)
                                    {
                                        let path = col.path().ok_or_else(|| {
                                            SofError::InvalidViewDefinition(
                                                "Column path is required".to_string(),
                                            )
                                        })?;

                                        let result = if path == "$this" {
                                            item.clone()
                                        } else {
                                            evaluate_path_on_item(path, item, variables)?
                                        };

                                        // Check if this column is marked as a collection
                                        let is_collection = col.collection().unwrap_or(false);

                                        base_combo.values[col_index] = if is_collection {
                                            fhirpath_result_to_json_value_collection(result)
                                        } else {
                                            fhirpath_result_to_json_value(result)
                                        };
                                    }
                                }
                            }
                        }
                    }
                }

                // Process each unionAll select independently for this iteration item
                for union_select in union_selects {
                    let mut select_combinations = vec![base_combo.clone()];
                    select_combinations = expand_select_combinations(
                        &item_context,
                        union_select,
                        &select_combinations,
                        all_columns,
                        variables,
                    )?;
                    union_combinations.extend(select_combinations);
                }
            }
        }

        // unionAll replaces new_combinations with the union results
        // If no union results, filter out this resource (no rows for this resource)
        new_combinations = union_combinations;
    }

    Ok(new_combinations)
}

// Generic helper functions
fn evaluate_path_on_item(
    path: &str,
    item: &EvaluationResult,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<EvaluationResult, SofError> {
    // Create a temporary context with the iteration item as the root resource
    let mut temp_context = match item {
        EvaluationResult::Object { .. } => {
            // Convert the iteration item to a resource-like structure for FHIRPath evaluation
            // For simplicity, we'll create a basic context where the item is available for evaluation
            let mut context = EvaluationContext::new(vec![]);
            context.this = Some(item.clone());
            context
        }
        _ => EvaluationContext::new(vec![]),
    };

    // Add variables to the temporary context
    for (name, value) in variables {
        temp_context.set_variable_result(name, value.clone());
    }

    // Evaluate the FHIRPath expression in the context of the iteration item
    match evaluate_expression(path, &temp_context) {
        Ok(result) => Ok(result),
        Err(_e) => {
            // If FHIRPath evaluation fails, try simple property access as fallback
            match item {
                EvaluationResult::Object { map, .. } => {
                    if let Some(value) = map.get(path) {
                        Ok(value.clone())
                    } else {
                        Ok(EvaluationResult::Empty)
                    }
                }
                _ => Ok(EvaluationResult::Empty),
            }
        }
    }
}

fn create_iteration_context(
    item: &EvaluationResult,
    variables: &HashMap<String, EvaluationResult>,
) -> EvaluationContext {
    // Create a new context with the iteration item as the root
    let mut context = EvaluationContext::new(vec![]);
    context.this = Some(item.clone());

    // Preserve variables from the parent context
    for (name, value) in variables {
        context.set_variable_result(name, value.clone());
    }

    context
}

/// Filter a bundle's resources by their lastUpdated metadata
fn filter_bundle_by_since(bundle: SofBundle, since: DateTime<Utc>) -> Result<SofBundle, SofError> {
    match bundle {
        #[cfg(feature = "R4")]
        SofBundle::R4(mut b) => {
            if let Some(entries) = b.entry.as_mut() {
                entries.retain(|entry| {
                    entry
                        .resource
                        .as_ref()
                        .and_then(|r| r.get_last_updated())
                        .map(|last_updated| last_updated > since)
                        .unwrap_or(false)
                });
            }
            Ok(SofBundle::R4(b))
        }
        #[cfg(feature = "R4B")]
        SofBundle::R4B(mut b) => {
            if let Some(entries) = b.entry.as_mut() {
                entries.retain(|entry| {
                    entry
                        .resource
                        .as_ref()
                        .and_then(|r| r.get_last_updated())
                        .map(|last_updated| last_updated > since)
                        .unwrap_or(false)
                });
            }
            Ok(SofBundle::R4B(b))
        }
        #[cfg(feature = "R5")]
        SofBundle::R5(mut b) => {
            if let Some(entries) = b.entry.as_mut() {
                entries.retain(|entry| {
                    entry
                        .resource
                        .as_ref()
                        .and_then(|r| r.get_last_updated())
                        .map(|last_updated| last_updated > since)
                        .unwrap_or(false)
                });
            }
            Ok(SofBundle::R5(b))
        }
        #[cfg(feature = "R6")]
        SofBundle::R6(mut b) => {
            if let Some(entries) = b.entry.as_mut() {
                entries.retain(|entry| {
                    entry
                        .resource
                        .as_ref()
                        .and_then(|r| r.get_last_updated())
                        .map(|last_updated| last_updated > since)
                        .unwrap_or(false)
                });
            }
            Ok(SofBundle::R6(b))
        }
    }
}

/// Apply pagination to processed results
fn apply_pagination_to_result(
    mut result: ProcessedResult,
    limit: Option<usize>,
    page: Option<usize>,
) -> Result<ProcessedResult, SofError> {
    if let Some(limit) = limit {
        let page_num = page.unwrap_or(1);
        if page_num == 0 {
            return Err(SofError::InvalidViewDefinition(
                "Page number must be greater than 0".to_string(),
            ));
        }

        let start_index = (page_num - 1) * limit;
        if start_index >= result.rows.len() {
            // Return empty result if page is beyond data
            result.rows.clear();
        } else {
            let end_index = std::cmp::min(start_index + limit, result.rows.len());
            result.rows = result.rows[start_index..end_index].to_vec();
        }
    }

    Ok(result)
}

fn format_output(
    result: ProcessedResult,
    content_type: ContentType,
    parquet_options: Option<&ParquetOptions>,
) -> Result<Vec<u8>, SofError> {
    match content_type {
        ContentType::Csv | ContentType::CsvWithHeader => {
            format_csv(result, content_type == ContentType::CsvWithHeader)
        }
        ContentType::Json => format_json(result),
        ContentType::NdJson => format_ndjson(result),
        ContentType::Parquet => format_parquet(result, parquet_options),
    }
}

fn format_csv(result: ProcessedResult, include_header: bool) -> Result<Vec<u8>, SofError> {
    let mut wtr = csv::Writer::from_writer(vec![]);

    if include_header {
        wtr.write_record(&result.columns)?;
    }

    for row in result.rows {
        let record: Vec<String> = row
            .values
            .iter()
            .map(|v| match v {
                Some(val) => {
                    // For string values, extract the raw string instead of JSON serializing
                    if let serde_json::Value::String(s) = val {
                        s.clone()
                    } else {
                        // For non-string values, use JSON serialization
                        serde_json::to_string(val).unwrap_or_default()
                    }
                }
                None => String::new(),
            })
            .collect();
        wtr.write_record(&record)?;
    }

    wtr.into_inner()
        .map_err(|e| SofError::CsvWriterError(e.to_string()))
}

fn format_json(result: ProcessedResult) -> Result<Vec<u8>, SofError> {
    let mut output = Vec::new();

    for row in result.rows {
        let mut row_obj = serde_json::Map::new();
        for (i, column) in result.columns.iter().enumerate() {
            let value = row
                .values
                .get(i)
                .and_then(|v| v.as_ref())
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            row_obj.insert(column.clone(), value);
        }
        output.push(serde_json::Value::Object(row_obj));
    }

    Ok(serde_json::to_vec_pretty(&output)?)
}

fn format_ndjson(result: ProcessedResult) -> Result<Vec<u8>, SofError> {
    let mut output = Vec::new();

    for row in result.rows {
        let mut row_obj = serde_json::Map::new();
        for (i, column) in result.columns.iter().enumerate() {
            let value = row
                .values
                .get(i)
                .and_then(|v| v.as_ref())
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            row_obj.insert(column.clone(), value);
        }
        let line = serde_json::to_string(&serde_json::Value::Object(row_obj))?;
        output.extend_from_slice(line.as_bytes());
        output.push(b'\n');
    }

    Ok(output)
}

fn format_parquet(
    result: ProcessedResult,
    options: Option<&ParquetOptions>,
) -> Result<Vec<u8>, SofError> {
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;
    use std::io::Cursor;

    // Create Arrow schema from columns and sample data
    let schema = parquet_schema::create_arrow_schema(&result.columns, &result.rows)?;
    let schema_ref = std::sync::Arc::new(schema.clone());

    // Get configuration from options or use defaults
    let parquet_opts = options.cloned().unwrap_or_default();

    // Calculate optimal batch size based on row count and estimated row size
    let target_row_group_size_bytes = (parquet_opts.row_group_size_mb as usize) * 1024 * 1024;
    let target_page_size_bytes = (parquet_opts.page_size_kb as usize) * 1024;
    const TARGET_ROWS_PER_BATCH: usize = 100_000; // Default batch size
    const MAX_ROWS_PER_BATCH: usize = 500_000; // Maximum to prevent memory issues

    // Estimate average row size from first 100 rows
    let sample_size = std::cmp::min(100, result.rows.len());
    let mut estimated_row_size = 100; // Default estimate in bytes

    if sample_size > 0 {
        let sample_json_size: usize = result.rows[..sample_size]
            .iter()
            .map(|row| {
                row.values
                    .iter()
                    .filter_map(|v| v.as_ref())
                    .map(|v| v.to_string().len())
                    .sum::<usize>()
            })
            .sum();
        estimated_row_size = (sample_json_size / sample_size).max(50);
    }

    // Calculate optimal batch size
    let optimal_batch_size = (target_row_group_size_bytes / estimated_row_size)
        .clamp(TARGET_ROWS_PER_BATCH, MAX_ROWS_PER_BATCH);

    // Parse compression algorithm
    use parquet::basic::BrotliLevel;
    use parquet::basic::GzipLevel;
    use parquet::basic::ZstdLevel;

    let compression = match parquet_opts.compression.as_str() {
        "none" => Compression::UNCOMPRESSED,
        "gzip" => Compression::GZIP(GzipLevel::default()),
        "lz4" => Compression::LZ4,
        "brotli" => Compression::BROTLI(BrotliLevel::default()),
        "zstd" => Compression::ZSTD(ZstdLevel::default()),
        _ => Compression::SNAPPY, // Default to snappy
    };

    // Set up writer properties with optimized settings
    let props = WriterProperties::builder()
        .set_compression(compression)
        .set_max_row_group_size(target_row_group_size_bytes)
        .set_data_page_row_count_limit(20_000) // Optimal for predicate pushdown
        .set_data_page_size_limit(target_page_size_bytes)
        .set_write_batch_size(8192) // Control write granularity
        .build();

    // Write to memory buffer
    let mut buffer = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);
    let mut writer =
        ArrowWriter::try_new(&mut cursor, schema_ref.clone(), Some(props)).map_err(|e| {
            SofError::ParquetConversionError(format!("Failed to create Parquet writer: {}", e))
        })?;

    // Process data in batches to handle large datasets efficiently
    let mut row_offset = 0;
    while row_offset < result.rows.len() {
        let batch_end = (row_offset + optimal_batch_size).min(result.rows.len());
        let batch_rows = &result.rows[row_offset..batch_end];

        // Convert batch to Arrow arrays
        let batch_arrays =
            parquet_schema::process_to_arrow_arrays(&schema, &result.columns, batch_rows)?;

        // Create RecordBatch for this chunk
        let batch = RecordBatch::try_new(schema_ref.clone(), batch_arrays).map_err(|e| {
            SofError::ParquetConversionError(format!(
                "Failed to create RecordBatch for rows {}-{}: {}",
                row_offset, batch_end, e
            ))
        })?;

        // Write batch
        writer.write(&batch).map_err(|e| {
            SofError::ParquetConversionError(format!(
                "Failed to write RecordBatch for rows {}-{}: {}",
                row_offset, batch_end, e
            ))
        })?;

        row_offset = batch_end;
    }

    writer.close().map_err(|e| {
        SofError::ParquetConversionError(format!("Failed to close Parquet writer: {}", e))
    })?;

    Ok(buffer)
}

/// Format Parquet data with automatic file splitting when size exceeds limit
pub fn format_parquet_multi_file(
    result: ProcessedResult,
    options: Option<&ParquetOptions>,
    max_file_size_bytes: usize,
) -> Result<Vec<Vec<u8>>, SofError> {
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;
    use std::io::Cursor;

    // Create Arrow schema from columns and sample data
    let schema = parquet_schema::create_arrow_schema(&result.columns, &result.rows)?;
    let schema_ref = std::sync::Arc::new(schema.clone());

    // Get configuration from options or use defaults
    let parquet_opts = options.cloned().unwrap_or_default();

    // Calculate optimal batch size
    let target_row_group_size_bytes = (parquet_opts.row_group_size_mb as usize) * 1024 * 1024;
    let target_page_size_bytes = (parquet_opts.page_size_kb as usize) * 1024;
    const TARGET_ROWS_PER_BATCH: usize = 100_000;
    const MAX_ROWS_PER_BATCH: usize = 500_000;

    // Estimate average row size
    let sample_size = std::cmp::min(100, result.rows.len());
    let mut estimated_row_size = 100;

    if sample_size > 0 {
        let sample_json_size: usize = result.rows[..sample_size]
            .iter()
            .map(|row| {
                row.values
                    .iter()
                    .filter_map(|v| v.as_ref())
                    .map(|v| v.to_string().len())
                    .sum::<usize>()
            })
            .sum();
        estimated_row_size = (sample_json_size / sample_size).max(50);
    }

    let optimal_batch_size = (target_row_group_size_bytes / estimated_row_size)
        .clamp(TARGET_ROWS_PER_BATCH, MAX_ROWS_PER_BATCH);

    // Parse compression algorithm
    use parquet::basic::BrotliLevel;
    use parquet::basic::GzipLevel;
    use parquet::basic::ZstdLevel;

    let compression = match parquet_opts.compression.as_str() {
        "none" => Compression::UNCOMPRESSED,
        "gzip" => Compression::GZIP(GzipLevel::default()),
        "lz4" => Compression::LZ4,
        "brotli" => Compression::BROTLI(BrotliLevel::default()),
        "zstd" => Compression::ZSTD(ZstdLevel::default()),
        _ => Compression::SNAPPY,
    };

    // Set up writer properties
    let props = WriterProperties::builder()
        .set_compression(compression)
        .set_max_row_group_size(target_row_group_size_bytes)
        .set_data_page_row_count_limit(20_000)
        .set_data_page_size_limit(target_page_size_bytes)
        .set_write_batch_size(8192)
        .build();

    let mut file_buffers = Vec::new();
    let mut current_buffer = Vec::new();
    let mut current_cursor = Cursor::new(&mut current_buffer);
    let mut current_writer =
        ArrowWriter::try_new(&mut current_cursor, schema_ref.clone(), Some(props.clone()))
            .map_err(|e| {
                SofError::ParquetConversionError(format!("Failed to create Parquet writer: {}", e))
            })?;

    let mut row_offset = 0;
    let mut _current_file_rows = 0;

    while row_offset < result.rows.len() {
        let batch_end = (row_offset + optimal_batch_size).min(result.rows.len());
        let batch_rows = &result.rows[row_offset..batch_end];

        // Convert batch to Arrow arrays
        let batch_arrays =
            parquet_schema::process_to_arrow_arrays(&schema, &result.columns, batch_rows)?;

        // Create RecordBatch
        let batch = RecordBatch::try_new(schema_ref.clone(), batch_arrays).map_err(|e| {
            SofError::ParquetConversionError(format!(
                "Failed to create RecordBatch for rows {}-{}: {}",
                row_offset, batch_end, e
            ))
        })?;

        // Write batch
        current_writer.write(&batch).map_err(|e| {
            SofError::ParquetConversionError(format!(
                "Failed to write RecordBatch for rows {}-{}: {}",
                row_offset, batch_end, e
            ))
        })?;

        _current_file_rows += batch_end - row_offset;
        row_offset = batch_end;

        // Check if we should start a new file
        // Get actual size of current buffer by flushing the writer
        let current_size = current_writer.bytes_written();

        if current_size >= max_file_size_bytes && row_offset < result.rows.len() {
            // Close current file
            current_writer.close().map_err(|e| {
                SofError::ParquetConversionError(format!("Failed to close Parquet writer: {}", e))
            })?;

            // Save the buffer
            file_buffers.push(current_buffer);

            // Start new file
            current_buffer = Vec::new();
            current_cursor = Cursor::new(&mut current_buffer);
            current_writer =
                ArrowWriter::try_new(&mut current_cursor, schema_ref.clone(), Some(props.clone()))
                    .map_err(|e| {
                        SofError::ParquetConversionError(format!(
                            "Failed to create new Parquet writer: {}",
                            e
                        ))
                    })?;
            _current_file_rows = 0;
        }
    }

    // Close the final writer
    current_writer.close().map_err(|e| {
        SofError::ParquetConversionError(format!("Failed to close final Parquet writer: {}", e))
    })?;

    file_buffers.push(current_buffer);

    Ok(file_buffers)
}
