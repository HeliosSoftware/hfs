//! Version-independent FHIR Parameters resource wrapper
//!
//! This module provides a unified interface for working with FHIR Parameters
//! resources across different specification versions. It enables applications
//! to handle multiple FHIR versions simultaneously while maintaining type safety.

use crate::FhirVersion;
use serde::{Deserialize, Serialize};

/// Multi-version Parameters container for version-agnostic operations.
///
/// This enum provides a unified interface for working with Parameters resources
/// across different FHIR specification versions. It enables applications to handle
/// multiple FHIR versions simultaneously while maintaining type safety and
/// version-specific behavior where needed.
///
/// # Supported Versions
///
/// - **R4**: FHIR 4.0.1 Parameters (normative)
/// - **R4B**: FHIR 4.3.0 Parameters (ballot)
/// - **R5**: FHIR 5.0.0 Parameters (ballot)
/// - **R6**: FHIR 6.0.0 Parameters (draft)
///
/// # Examples
///
/// ```rust
/// use helios_fhir::VersionIndependentParameters;
/// # #[cfg(feature = "R4")]
/// use helios_fhir::r4::Parameters;
///
/// # #[cfg(feature = "R4")]
/// # {
/// // Parse from JSON
/// let json = r#"{
///     "resourceType": "Parameters",
///     "parameter": [{
///         "name": "expression",
///         "valueString": "Patient.name"
///     }]
/// }"#;
///
/// let params: Parameters = serde_json::from_str(json)?;
/// let version_independent = VersionIndependentParameters::R4(params);
///
/// // Check version
/// assert_eq!(version_independent.version(), helios_fhir::FhirVersion::R4);
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum VersionIndependentParameters {
    /// FHIR 4.0.1 Parameters
    #[cfg(feature = "R4")]
    R4(crate::r4::Parameters),
    /// FHIR 4.3.0 Parameters
    #[cfg(feature = "R4B")]
    R4B(crate::r4b::Parameters),
    /// FHIR 5.0.0 Parameters
    #[cfg(feature = "R5")]
    R5(crate::r5::Parameters),
    /// FHIR 6.0.0 Parameters
    #[cfg(feature = "R6")]
    R6(crate::r6::Parameters),
}

impl VersionIndependentParameters {
    /// Returns the FHIR specification version of this Parameters resource.
    ///
    /// This method provides version detection for multi-version applications,
    /// enabling version-specific processing logic and compatibility checks.
    ///
    /// # Returns
    ///
    /// The `FhirVersion` enum variant corresponding to this Parameters resource.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use helios_fhir::{VersionIndependentParameters, FhirVersion};
    ///
    /// # #[cfg(feature = "R4")]
    /// # {
    /// # let params = helios_fhir::r4::Parameters::default();
    /// let version_independent = VersionIndependentParameters::R4(params);
    /// assert_eq!(version_independent.version(), FhirVersion::R4);
    /// # }
    /// ```
    pub fn version(&self) -> FhirVersion {
        match self {
            #[cfg(feature = "R4")]
            VersionIndependentParameters::R4(_) => FhirVersion::R4,
            #[cfg(feature = "R4B")]
            VersionIndependentParameters::R4B(_) => FhirVersion::R4B,
            #[cfg(feature = "R5")]
            VersionIndependentParameters::R5(_) => FhirVersion::R5,
            #[cfg(feature = "R6")]
            VersionIndependentParameters::R6(_) => FhirVersion::R6,
        }
    }

    /// Converts the Parameters to a JSON Value for version-independent processing.
    ///
    /// This method is useful when you need to process parameters in a
    /// version-agnostic way using JSON traversal.
    ///
    /// # Returns
    ///
    /// A `serde_json::Value` representation of the Parameters resource.
    ///
    /// # Errors
    ///
    /// Returns an error if JSON serialization fails.
    pub fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

/// Trait for accessing parameter values in a version-independent way.
///
/// This trait provides a common interface for extracting values from
/// ParametersParameterValue across different FHIR versions.
pub trait ParameterValueAccessor {
    /// Extracts a string value if the parameter contains valueString.
    fn as_string(&self) -> Option<&str>;

    /// Extracts a boolean value if the parameter contains valueBoolean.
    fn as_boolean(&self) -> Option<bool>;

    /// Extracts an integer value if the parameter contains valueInteger.
    fn as_integer(&self) -> Option<i64>;

    /// Extracts a decimal value if the parameter contains valueDecimal.
    fn as_decimal(&self) -> Option<&crate::PreciseDecimal>;
}

#[cfg(feature = "R4")]
impl ParameterValueAccessor for crate::r4::ParametersParameterValue {
    fn as_string(&self) -> Option<&str> {
        match self {
            crate::r4::ParametersParameterValue::String(s) => s.value.as_deref(),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self {
            crate::r4::ParametersParameterValue::Boolean(b) => b.value,
            _ => None,
        }
    }

    fn as_integer(&self) -> Option<i64> {
        match self {
            crate::r4::ParametersParameterValue::Integer(i) => i.value.map(|v| v as i64),
            _ => None,
        }
    }

    fn as_decimal(&self) -> Option<&crate::PreciseDecimal> {
        match self {
            crate::r4::ParametersParameterValue::Decimal(d) => d.value.as_ref(),
            _ => None,
        }
    }
}

#[cfg(feature = "R4B")]
impl ParameterValueAccessor for crate::r4b::ParametersParameterValue {
    fn as_string(&self) -> Option<&str> {
        match self {
            crate::r4b::ParametersParameterValue::String(s) => s.value.as_deref(),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self {
            crate::r4b::ParametersParameterValue::Boolean(b) => b.value,
            _ => None,
        }
    }

    fn as_integer(&self) -> Option<i64> {
        match self {
            crate::r4b::ParametersParameterValue::Integer(i) => i.value.map(|v| v as i64),
            _ => None,
        }
    }

    fn as_decimal(&self) -> Option<&crate::PreciseDecimal> {
        match self {
            crate::r4b::ParametersParameterValue::Decimal(d) => d.value.as_ref(),
            _ => None,
        }
    }
}

#[cfg(feature = "R5")]
impl ParameterValueAccessor for crate::r5::ParametersParameterValue {
    fn as_string(&self) -> Option<&str> {
        match self {
            crate::r5::ParametersParameterValue::String(s) => s.value.as_deref(),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self {
            crate::r5::ParametersParameterValue::Boolean(b) => b.value,
            _ => None,
        }
    }

    fn as_integer(&self) -> Option<i64> {
        match self {
            crate::r5::ParametersParameterValue::Integer(i) => i.value.map(|v| v as i64),
            _ => None,
        }
    }

    fn as_decimal(&self) -> Option<&crate::PreciseDecimal> {
        match self {
            crate::r5::ParametersParameterValue::Decimal(d) => d.value.as_ref(),
            _ => None,
        }
    }
}

#[cfg(feature = "R6")]
impl ParameterValueAccessor for crate::r6::ParametersParameterValue {
    fn as_string(&self) -> Option<&str> {
        match self {
            crate::r6::ParametersParameterValue::String(s) => s.value.as_deref(),
            _ => None,
        }
    }

    fn as_boolean(&self) -> Option<bool> {
        match self {
            crate::r6::ParametersParameterValue::Boolean(b) => b.value,
            _ => None,
        }
    }

    fn as_integer(&self) -> Option<i64> {
        match self {
            crate::r6::ParametersParameterValue::Integer(i) => i.value.map(|v| v as i64),
            _ => None,
        }
    }

    fn as_decimal(&self) -> Option<&crate::PreciseDecimal> {
        match self {
            crate::r6::ParametersParameterValue::Decimal(d) => d.value.as_ref(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "R4")]
    fn test_version_detection() {
        let params = crate::r4::Parameters::default();
        let version_independent = VersionIndependentParameters::R4(params);
        assert_eq!(version_independent.version(), FhirVersion::R4);
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_json_conversion() {
        let params = crate::r4::Parameters::default();
        let version_independent = VersionIndependentParameters::R4(params);
        let json = version_independent.to_json().unwrap();
        // The resourceType is added during serialization via serde attributes
        // so we'll test that we can convert to JSON without errors
        assert!(json.is_object());
    }
}
