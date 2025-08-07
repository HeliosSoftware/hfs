//! Data models for FHIRPath server request and response handling
//!
//! This module defines the structures used for the FHIRPath server API,
//! following the specification in server-api.md for the fhirpath-lab
//! integration.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use helios_fhir::FhirVersion;

// Helper trait for accessing values from ParametersParameterValue
trait ParameterValueAccessor {
    fn as_string(&self) -> Option<&str>;
    fn as_boolean(&self) -> Option<bool>;
}

#[cfg(feature = "R4")]
impl ParameterValueAccessor for helios_fhir::r4::ParametersParameterValue {
    fn as_string(&self) -> Option<&str> {
        match self {
            helios_fhir::r4::ParametersParameterValue::String(s) => 
                s.value.as_ref().map(|v| v.as_str()),
            _ => None,
        }
    }
    
    fn as_boolean(&self) -> Option<bool> {
        match self {
            helios_fhir::r4::ParametersParameterValue::Boolean(b) => 
                b.value,
            _ => None,
        }
    }
}

#[cfg(feature = "R4B")]
impl ParameterValueAccessor for helios_fhir::r4b::ParametersParameterValue {
    fn as_string(&self) -> Option<&str> {
        match self {
            helios_fhir::r4b::ParametersParameterValue::String(s) => 
                s.value.as_ref().map(|v| v.as_str()),
            _ => None,
        }
    }
    
    fn as_boolean(&self) -> Option<bool> {
        match self {
            helios_fhir::r4b::ParametersParameterValue::Boolean(b) => 
                b.value,
            _ => None,
        }
    }
}

#[cfg(feature = "R5")]
impl ParameterValueAccessor for helios_fhir::r5::ParametersParameterValue {
    fn as_string(&self) -> Option<&str> {
        match self {
            helios_fhir::r5::ParametersParameterValue::String(s) => 
                s.value.as_ref().map(|v| v.as_str()),
            _ => None,
        }
    }
    
    fn as_boolean(&self) -> Option<bool> {
        match self {
            helios_fhir::r5::ParametersParameterValue::Boolean(b) => 
                b.value,
            _ => None,
        }
    }
}

#[cfg(feature = "R6")]
impl ParameterValueAccessor for helios_fhir::r6::ParametersParameterValue {
    fn as_string(&self) -> Option<&str> {
        match self {
            helios_fhir::r6::ParametersParameterValue::String(s) => 
                s.value.as_ref().map(|v| v.as_str()),
            _ => None,
        }
    }
    
    fn as_boolean(&self) -> Option<bool> {
        match self {
            helios_fhir::r6::ParametersParameterValue::Boolean(b) => 
                b.value,
            _ => None,
        }
    }
}

/// Multi-version Parameters container for FHIRPath operations
///
/// This enum provides a unified interface for working with Parameters resources
/// across different FHIR specification versions. It enables the FHIRPath server
/// to handle requests from clients using any supported FHIR version while
/// maintaining type safety.
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
/// ```rust,no_run
/// use helios_fhirpath::models::FhirPathParameters;
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
/// let fhirpath_params = FhirPathParameters::R4(params);
///
/// // Check version
/// assert_eq!(fhirpath_params.version(), helios_fhir::FhirVersion::R4);
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FhirPathParameters {
    #[cfg(feature = "R4")]
    R4(helios_fhir::r4::Parameters),
    #[cfg(feature = "R4B")]
    R4B(helios_fhir::r4b::Parameters),
    #[cfg(feature = "R5")]
    R5(helios_fhir::r5::Parameters),
    #[cfg(feature = "R6")]
    R6(helios_fhir::r6::Parameters),
}

impl FhirPathParameters {
    /// Returns the FHIR specification version of this Parameters resource
    ///
    /// This method provides version detection for multi-version applications,
    /// allowing the server to handle requests appropriately based on the
    /// FHIR version being used.
    ///
    /// # Returns
    ///
    /// The `FhirVersion` enum variant corresponding to this Parameters resource
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use helios_fhirpath::models::FhirPathParameters;
    /// use helios_fhir::FhirVersion;
    ///
    /// # #[cfg(feature = "R4")]
    /// # {
    /// # let params = helios_fhir::r4::Parameters::default();
    /// let fhirpath_params = FhirPathParameters::R4(params);
    /// assert_eq!(fhirpath_params.version(), FhirVersion::R4);
    /// # }
    /// ```
    pub fn version(&self) -> FhirVersion {
        match self {
            #[cfg(feature = "R4")]
            FhirPathParameters::R4(_) => FhirVersion::R4,
            #[cfg(feature = "R4B")]
            FhirPathParameters::R4B(_) => FhirVersion::R4B,
            #[cfg(feature = "R5")]
            FhirPathParameters::R5(_) => FhirVersion::R5,
            #[cfg(feature = "R6")]
            FhirPathParameters::R6(_) => FhirVersion::R6,
        }
    }
}


/// Individual parameter in the Parameters resource
#[derive(Debug, Deserialize, Serialize)]
pub struct Parameter {
    /// Name of the parameter
    pub name: String,
    
    /// String value (for simple parameters)
    #[serde(rename = "valueString", skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    
    /// Boolean value
    #[serde(rename = "valueBoolean", skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<bool>,
    
    /// Resource value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Value>,
    
    /// Multi-part parameters (for variables)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<Vec<ParameterPart>>,
}

/// Part of a multi-part parameter
#[derive(Debug, Deserialize, Serialize)]
pub struct ParameterPart {
    /// Name of the part
    pub name: String,
    
    /// String value
    #[serde(rename = "valueString", skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    
    /// Any other value type
    #[serde(flatten)]
    pub value: Option<Value>,
}

/// Extracted parameters for processing
#[derive(Debug, Default)]
pub struct ExtractedParameters {
    /// The context expression to execute first
    pub context: Option<String>,
    
    /// The FHIRPath expression to execute
    pub expression: Option<String>,
    
    /// Whether to validate the expression
    pub validate: bool,
    
    /// Variables to pass to the expression
    pub variables: Vec<Variable>,
    
    /// The resource to execute against
    pub resource: Option<Value>,
    
    /// Terminology server URL
    pub terminology_server: Option<String>,
}

/// Variable definition
#[derive(Debug, Clone)]
pub struct Variable {
    /// Variable name
    pub name: String,
    
    /// Variable value
    pub value: Value,
}

/// Output result part
#[derive(Debug, Serialize)]
pub struct ResultPart {
    /// Name of the part (data type or "trace")
    pub name: String,
    
    /// String value for context path
    #[serde(rename = "valueString", skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    
    /// Parts for complex results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<Vec<ResultValue>>,
    
    /// Extension for non-representable values
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
}

/// Result value within a part
#[derive(Debug, Serialize)]
pub struct ResultValue {
    /// Name (data type)
    pub name: String,
    
    /// Various value types
    #[serde(rename = "valueString", skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    
    #[serde(rename = "valueBoolean", skip_serializing_if = "Option::is_none")]
    pub value_boolean: Option<bool>,
    
    #[serde(rename = "valueInteger", skip_serializing_if = "Option::is_none")]
    pub value_integer: Option<i64>,
    
    #[serde(rename = "valueDecimal", skip_serializing_if = "Option::is_none")]
    pub value_decimal: Option<f64>,
    
    #[serde(rename = "valueDate", skip_serializing_if = "Option::is_none")]
    pub value_date: Option<String>,
    
    #[serde(rename = "valueDateTime", skip_serializing_if = "Option::is_none")]
    pub value_date_time: Option<String>,
    
    #[serde(rename = "valueTime", skip_serializing_if = "Option::is_none")]
    pub value_time: Option<String>,
    
    #[serde(rename = "valueQuantity", skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Value>,
    
    #[serde(rename = "valueHumanName", skip_serializing_if = "Option::is_none")]
    pub value_human_name: Option<Value>,
    
    /// Extension for JSON representation of complex values
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<Vec<Extension>>,
}

/// Extension for JSON values that can't be represented as FHIR types
#[derive(Debug, Serialize)]
pub struct Extension {
    /// Extension URL
    pub url: String,
    
    /// String value containing JSON
    #[serde(rename = "valueString")]
    pub value_string: String,
}

/// Helper to create JSON value extension
pub fn create_json_extension(value: &Value) -> Extension {
    Extension {
        url: "http://fhir.forms-lab.com/StructureDefinition/json-value".to_string(),
        value_string: serde_json::to_string_pretty(value).unwrap_or_default(),
    }
}

/// Extract parameters from the input Parameters resource
pub fn extract_parameters(params: FhirPathParameters) -> Result<ExtractedParameters, String> {
    let mut extracted = ExtractedParameters::default();
    
    // Process parameters based on version
    match params {
        #[cfg(feature = "R4")]
        FhirPathParameters::R4(parameters) => {
            extract_parameters_from_r4(parameters, &mut extracted)?;
        }
        #[cfg(feature = "R4B")]
        FhirPathParameters::R4B(parameters) => {
            extract_parameters_from_r4b(parameters, &mut extracted)?;
        }
        #[cfg(feature = "R5")]
        FhirPathParameters::R5(parameters) => {
            extract_parameters_from_r5(parameters, &mut extracted)?;
        }
        #[cfg(feature = "R6")]
        FhirPathParameters::R6(parameters) => {
            extract_parameters_from_r6(parameters, &mut extracted)?;
        }
    }
    
    if extracted.expression.is_none() {
        return Err("Missing required parameter: expression".to_string());
    }
    
    if extracted.resource.is_none() {
        return Err("Missing required parameter: resource".to_string());
    }
    
    Ok(extracted)
}

#[cfg(feature = "R4")]
fn extract_parameters_from_r4(
    parameters: helios_fhir::r4::Parameters,
    extracted: &mut ExtractedParameters,
) -> Result<(), String> {
    
    for param in parameters.parameter.unwrap_or_default() {
        process_parameter_r4(&param, extracted)?;
    }
    Ok(())
}

#[cfg(feature = "R4")]
fn process_parameter_r4(
    param: &helios_fhir::r4::ParametersParameter,
    extracted: &mut ExtractedParameters,
) -> Result<(), String> {
    let name = param.name.value.as_ref().map(|s| s.as_str()).unwrap_or("");
    
    match name {
        "context" => {
            extracted.context = param.value.as_ref()
                .and_then(|v| v.as_string())
                .map(|s| s.to_string());
        }
        "expression" => {
            extracted.expression = param.value.as_ref()
                .and_then(|v| v.as_string())
                .map(|s| s.to_string());
        }
        "validate" => {
            extracted.validate = param.value.as_ref()
                .and_then(|v| v.as_boolean())
                .unwrap_or(false);
        }
        "variables" => {
            if let Some(parts) = &param.part {
                for part in parts {
                    if let Some(name) = &part.name.value {
                        let value = if let Some(val) = &part.value {
                            // Convert parameter value to JSON
                            parameter_value_to_json_r4(val)
                        } else {
                            Value::Null
                        };
                        
                        extracted.variables.push(Variable {
                            name: name.to_string(),
                            value,
                        });
                    }
                }
            }
        }
        "resource" => {
            extracted.resource = param.resource.as_ref()
                .and_then(|r| serde_json::to_value(r).ok());
        }
        "terminologyServer" => {
            extracted.terminology_server = param.value.as_ref()
                .and_then(|v| v.as_string())
                .map(|s| s.to_string());
        }
        _ => {
            // Ignore unknown parameters
        }
    }
    
    Ok(())
}

#[cfg(feature = "R4")]
fn parameter_value_to_json_r4(value: &helios_fhir::r4::ParametersParameterValue) -> Value {
    // Convert FHIR parameter value to JSON
    // This is a simplified conversion - in production, you'd handle all value types
    match value {
        helios_fhir::r4::ParametersParameterValue::String(s) => {
            s.value.as_ref().map(|v| Value::String(v.clone())).unwrap_or(Value::Null)
        },
        helios_fhir::r4::ParametersParameterValue::Boolean(b) => {
            b.value.map(Value::Bool).unwrap_or(Value::Null)
        },
        helios_fhir::r4::ParametersParameterValue::Integer(i) => {
            i.value.map(|v| Value::Number(serde_json::Number::from(v))).unwrap_or(Value::Null)
        }
        helios_fhir::r4::ParametersParameterValue::Decimal(d) => {
            serde_json::to_value(d).unwrap_or(Value::Null)
        }
        _ => {
            // For other types, serialize to JSON
            serde_json::to_value(value).unwrap_or(Value::Null)
        }
    }
}

#[cfg(feature = "R4B")]
fn extract_parameters_from_r4b(
    parameters: helios_fhir::r4b::Parameters,
    extracted: &mut ExtractedParameters,
) -> Result<(), String> {
    // Similar implementation for R4B
    // For brevity, this follows the same pattern as R4
    for param in parameters.parameter.unwrap_or_default() {
        let name = param.name.value.as_ref().map(|s| s.as_str()).unwrap_or("");
        
        match name {
            "context" => {
                extracted.context = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            "expression" => {
                extracted.expression = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            "validate" => {
                extracted.validate = param.value.as_ref()
                    .and_then(|v| v.as_boolean())
                    .unwrap_or(false);
            }
            "resource" => {
                extracted.resource = param.resource.as_ref()
                    .and_then(|r| serde_json::to_value(r).ok());
            }
            "terminologyServer" => {
                extracted.terminology_server = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            // Handle variables similarly
            _ => {}
        }
    }
    Ok(())
}

#[cfg(feature = "R5")]
fn extract_parameters_from_r5(
    parameters: helios_fhir::r5::Parameters,
    extracted: &mut ExtractedParameters,
) -> Result<(), String> {
    // Similar implementation for R5
    for param in parameters.parameter.unwrap_or_default() {
        let name = param.name.value.as_ref().map(|s| s.as_str()).unwrap_or("");
        
        match name {
            "context" => {
                extracted.context = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            "expression" => {
                extracted.expression = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            "validate" => {
                extracted.validate = param.value.as_ref()
                    .and_then(|v| v.as_boolean())
                    .copied()
                    .unwrap_or(false);
            }
            "resource" => {
                extracted.resource = param.resource.as_ref()
                    .and_then(|r| serde_json::to_value(r).ok());
            }
            "terminologyServer" => {
                extracted.terminology_server = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            // Handle variables similarly
            _ => {}
        }
    }
    Ok(())
}

#[cfg(feature = "R6")]
fn extract_parameters_from_r6(
    parameters: helios_fhir::r6::Parameters,
    extracted: &mut ExtractedParameters,
) -> Result<(), String> {
    // Similar implementation for R6
    for param in parameters.parameter.unwrap_or_default() {
        let name = param.name.value.as_ref().map(|s| s.as_str()).unwrap_or("");
        
        match name {
            "context" => {
                extracted.context = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            "expression" => {
                extracted.expression = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            "validate" => {
                extracted.validate = param.value.as_ref()
                    .and_then(|v| v.as_boolean())
                    .copied()
                    .unwrap_or(false);
            }
            "resource" => {
                extracted.resource = param.resource.as_ref()
                    .and_then(|r| serde_json::to_value(r).ok());
            }
            "terminologyServer" => {
                extracted.terminology_server = param.value.as_ref()
                    .and_then(|v| v.as_string())
                    .map(|s| s.to_string());
            }
            // Handle variables similarly
            _ => {}
        }
    }
    Ok(())
}