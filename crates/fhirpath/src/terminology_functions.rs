//! Terminology functions for FHIRPath %terminologies object
//!
//! This module implements the %terminologies functions defined in the FHIRPath specification,
//! enabling interaction with FHIR terminology servers for ValueSet expansion, code validation,
//! and concept mapping operations.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::{Handle, Runtime};

use serde_json::Value;

use crate::evaluator::EvaluationContext;
use crate::terminology_client::TerminologyClient;
use helios_fhirpath_support::{EvaluationError, EvaluationResult};

lazy_static::lazy_static! {
    /// Lazy static for async runtime
    /// Used to execute async terminology operations in sync context
    static ref RUNTIME: Runtime = Runtime::new().expect("Failed to create tokio runtime");
}

/// Helper function to execute async operations in both sync and async contexts
fn block_on_async<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    // Check if we're already in an async runtime context
    if Handle::try_current().is_ok() {
        // We're in an async context, use block_in_place to handle the blocking operation
        // This temporarily removes the current task from the async runtime to avoid blocking
        tokio::task::block_in_place(move || {
            // Create a new single-threaded runtime for this blocking operation
            let rt = Runtime::new().expect("Failed to create runtime for terminology operation");
            rt.block_on(future)
        })
    } else {
        // Not in async context, safe to use the static runtime directly
        RUNTIME.block_on(future)
    }
}

/// Terminology functions accessible via %terminologies
pub struct TerminologyFunctions {
    client: Arc<TerminologyClient>,
}

impl TerminologyFunctions {
    /// Creates a new terminology functions instance
    pub fn new(context: &EvaluationContext) -> Self {
        let server_url = context.get_terminology_server_url();
        let client = TerminologyClient::new(server_url, context.fhir_version);

        Self {
            client: Arc::new(client),
        }
    }

    /// Expands a ValueSet
    ///
    /// Usage: %terminologies.expand(valueSet, params)
    pub fn expand(
        &self,
        value_set: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract ValueSet URL
        let value_set_url = match value_set {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "expand() requires a ValueSet URL as string".to_string(),
                ));
            }
        };

        // Extract parameters if provided
        let params_map = extract_params_map(params)?;

        // Execute async operation in blocking context
        let client = self.client.clone();
        let result = block_on_async(async move { client.expand(&value_set_url, params_map).await });

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::InvalidOperation(format!(
                "ValueSet expansion failed: {}",
                e
            ))),
        }
    }

    /// Looks up details for a code
    ///
    /// Usage: %terminologies.lookup(coded, params)
    pub fn lookup(
        &self,
        coded: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract system and code from Coding
        let (system, code) = extract_coding(coded)?;

        // Extract parameters if provided
        let params_map = extract_params_map(params)?;

        // Execute async operation
        let client = self.client.clone();
        let result = block_on_async(async move { client.lookup(&system, &code, params_map).await });

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::InvalidOperation(format!(
                "Code lookup failed: {}",
                e
            ))),
        }
    }

    /// Validates a code against a ValueSet
    ///
    /// Usage: %terminologies.validateVS(valueSet, coded, params)
    pub fn validate_vs(
        &self,
        value_set: &EvaluationResult,
        coded: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract ValueSet URL
        let value_set_url = match value_set {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "validateVS() requires a ValueSet URL as string".to_string(),
                ));
            }
        };

        // Extract coding information
        let (system, code, display) = extract_coding_with_display(coded)?;

        // Extract parameters
        let params_map = extract_params_map(params)?;

        // Execute async operation
        let client = self.client.clone();
        let system_opt = if system.is_empty() {
            None
        } else {
            Some(system.clone())
        };

        let result = block_on_async(async move {
            let system_ref = system_opt.as_deref();
            let display_ref = display.as_deref();
            client
                .validate_vs(&value_set_url, system_ref, &code, display_ref, params_map)
                .await
        });

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::InvalidOperation(format!(
                "ValueSet validation failed: {}",
                e
            ))),
        }
    }

    /// Validates a code against a CodeSystem
    ///
    /// Usage: %terminologies.validateCS(codeSystem, coded, params)
    pub fn validate_cs(
        &self,
        code_system: &EvaluationResult,
        coded: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract CodeSystem URL
        let code_system_url = match code_system {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "validateCS() requires a CodeSystem URL as string".to_string(),
                ));
            }
        };

        // Extract code and display
        let (_system, code, display) = extract_coding_with_display(coded)?;

        // Extract parameters
        let params_map = extract_params_map(params)?;

        // Execute async operation
        let client = self.client.clone();

        let result = block_on_async(async move {
            let display_ref = display.as_deref();
            client
                .validate_cs(&code_system_url, &code, display_ref, params_map)
                .await
        });

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::InvalidOperation(format!(
                "CodeSystem validation failed: {}",
                e
            ))),
        }
    }

    /// Checks if one code subsumes another
    ///
    /// Usage: %terminologies.subsumes(system, coded1, coded2, params)
    pub fn subsumes(
        &self,
        system: &EvaluationResult,
        coded1: &EvaluationResult,
        coded2: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract system URL
        let system_url = match system {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "subsumes() requires a system URL as string".to_string(),
                ));
            }
        };

        // Extract codes
        let (_sys1, code1) = extract_coding(coded1)?;
        let (_sys2, code2) = extract_coding(coded2)?;

        // Extract parameters
        let params_map = extract_params_map(params)?;

        // Execute async operation
        let client = self.client.clone();
        let result = block_on_async(async move {
            client
                .subsumes(&system_url, &code1, &code2, params_map)
                .await
        });

        match result {
            Ok(value) => {
                // Extract the 'outcome' parameter value
                if let Some(parameters) = value.get("parameter").and_then(|p| p.as_array()) {
                    for param in parameters {
                        if param.get("name").and_then(|n| n.as_str()) == Some("outcome") {
                            if let Some(code) = param.get("valueCode").and_then(|c| c.as_str()) {
                                return Ok(EvaluationResult::string(code.to_string()));
                            }
                        }
                    }
                }
                Err(EvaluationError::InvalidOperation(
                    "subsumes() result missing outcome parameter".to_string(),
                ))
            }
            Err(e) => Err(EvaluationError::InvalidOperation(format!(
                "Subsumes check failed: {}",
                e
            ))),
        }
    }

    /// Translates a code using a ConceptMap
    ///
    /// Usage: %terminologies.translate(conceptMap, code, params)
    pub fn translate(
        &self,
        concept_map: &EvaluationResult,
        code: &EvaluationResult,
        params: Option<&EvaluationResult>,
    ) -> Result<EvaluationResult, EvaluationError> {
        // Extract ConceptMap URL
        let concept_map_url = match concept_map {
            EvaluationResult::String(url, _, _) => url.clone(),
            _ => {
                return Err(EvaluationError::TypeError(
                    "translate() requires a ConceptMap URL as string".to_string(),
                ));
            }
        };

        // Extract coding
        let (system, code_str) = extract_coding(code)?;

        // Extract target system from params if provided
        let mut params_map = extract_params_map(params)?;
        let target_system = params_map.as_mut().and_then(|m| m.remove("targetSystem"));

        // Execute async operation
        let client = self.client.clone();

        let result = block_on_async(async move {
            let target_system_ref = target_system.as_deref();
            client
                .translate(
                    &concept_map_url,
                    &system,
                    &code_str,
                    target_system_ref,
                    params_map,
                )
                .await
        });

        match result {
            Ok(value) => json_to_evaluation_result(value),
            Err(e) => Err(EvaluationError::InvalidOperation(format!(
                "Translation failed: {}",
                e
            ))),
        }
    }
}

/// Extracts system and code from a Coding or CodeableConcept
fn extract_coding(coded: &EvaluationResult) -> Result<(String, String), EvaluationError> {
    match coded {
        // Direct code string
        EvaluationResult::String(code, _, _) => Ok((String::new(), code.clone())),

        // Coding object
        EvaluationResult::Object { map, .. } => {
            let system = map
                .get("system")
                .and_then(|v| match v {
                    EvaluationResult::String(s, _, _) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();

            let code = map
                .get("code")
                .and_then(|v| match v {
                    EvaluationResult::String(c, _, _) => Some(c.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    EvaluationError::TypeError("Coding must have a 'code' element".to_string())
                })?;

            Ok((system, code))
        }

        _ => Err(EvaluationError::TypeError(
            "Expected string code or Coding object".to_string(),
        )),
    }
}

/// Extracts system, code, and display from a Coding
fn extract_coding_with_display(
    coded: &EvaluationResult,
) -> Result<(String, String, Option<String>), EvaluationError> {
    match coded {
        // Direct code string
        EvaluationResult::String(code, _, _) => Ok((String::new(), code.clone(), None)),

        // Coding object
        EvaluationResult::Object { map, .. } => {
            let system = map
                .get("system")
                .and_then(|v| match v {
                    EvaluationResult::String(s, _, _) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_default();

            let code = map
                .get("code")
                .and_then(|v| match v {
                    EvaluationResult::String(c, _, _) => Some(c.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    EvaluationError::TypeError("Coding must have a 'code' element".to_string())
                })?;

            let display = map.get("display").and_then(|v| match v {
                EvaluationResult::String(d, _, _) => Some(d.clone()),
                _ => None,
            });

            Ok((system, code, display))
        }

        _ => Err(EvaluationError::TypeError(
            "Expected string code or Coding object".to_string(),
        )),
    }
}

/// Extracts parameters map from Parameters resource or object
fn extract_params_map(
    params: Option<&EvaluationResult>,
) -> Result<Option<HashMap<String, String>>, EvaluationError> {
    match params {
        None => Ok(None),
        Some(EvaluationResult::Object { map, .. }) => {
            let mut params_map = HashMap::new();

            // Check if it's a Parameters resource
            if let Some(EvaluationResult::Collection { items, .. }) = map.get("parameter") {
                // Extract parameters from Parameters resource format
                for item in items {
                    if let EvaluationResult::Object { map: param_map, .. } = item {
                        if let (Some(name), Some(value)) = (
                            param_map.get("name").and_then(|n| match n {
                                EvaluationResult::String(s, _, _) => Some(s),
                                _ => None,
                            }),
                            extract_parameter_value(param_map),
                        ) {
                            params_map.insert(name.clone(), value);
                        }
                    }
                }
            } else {
                // Treat as simple key-value map
                for (key, value) in map {
                    if let EvaluationResult::String(v, _, _) = value {
                        params_map.insert(key.clone(), v.clone());
                    }
                }
            }

            Ok(Some(params_map))
        }
        Some(_) => Err(EvaluationError::TypeError(
            "Parameters must be an object or Parameters resource".to_string(),
        )),
    }
}

/// Extracts value from a parameter element
fn extract_parameter_value(param_map: &HashMap<String, EvaluationResult>) -> Option<String> {
    // Check for various value[x] types
    for (key, value) in param_map {
        if key.starts_with("value") {
            match value {
                EvaluationResult::String(s, _, _) => return Some(s.clone()),
                EvaluationResult::Boolean(b, _, _) => return Some(b.to_string()),
                EvaluationResult::Integer(i, _, _) => return Some(i.to_string()),
                EvaluationResult::Decimal(d, _, _) => return Some(d.to_string()),
                _ => {}
            }
        }
    }
    None
}

/// Converts JSON Value to EvaluationResult
fn json_to_evaluation_result(value: Value) -> Result<EvaluationResult, EvaluationError> {
    match value {
        Value::Null => Ok(EvaluationResult::Empty),
        Value::Bool(b) => Ok(EvaluationResult::boolean(b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(EvaluationResult::integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(EvaluationResult::decimal(
                    rust_decimal::Decimal::from_f64_retain(f)
                        .unwrap_or(rust_decimal::Decimal::ZERO),
                ))
            } else {
                Ok(EvaluationResult::string(n.to_string()))
            }
        }
        Value::String(s) => Ok(EvaluationResult::string(s)),
        Value::Array(arr) => {
            let items: Result<Vec<_>, _> = arr.into_iter().map(json_to_evaluation_result).collect();
            Ok(EvaluationResult::Collection {
                items: items?,
                has_undefined_order: false,
                type_info: None,
            })
        }
        Value::Object(obj) => {
            let mut map = HashMap::new();
            for (key, val) in obj {
                map.insert(key, json_to_evaluation_result(val)?);
            }
            Ok(EvaluationResult::Object {
                map,
                type_info: None,
            })
        }
    }
}

/// memberOf function implementation for Coding/CodeableConcept
///
/// Usage: coding.memberOf(valueSetUrl)
pub fn member_of(
    coding: &EvaluationResult,
    value_set_url: &str,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    let terminology = TerminologyFunctions::new(context);

    // Call validateVS and extract the result
    let validation_result = terminology.validate_vs(
        &EvaluationResult::string(value_set_url.to_string()),
        coding,
        None,
    )?;

    // Extract the 'result' parameter from the Parameters response
    if let EvaluationResult::Object { map, .. } = validation_result {
        if let Some(EvaluationResult::Collection { items, .. }) = map.get("parameter") {
            for item in items {
                if let EvaluationResult::Object { map: param_map, .. } = item {
                    if param_map.get("name").and_then(|n| match n {
                        EvaluationResult::String(s, _, _) => Some(s.as_str()),
                        _ => None,
                    }) == Some("result")
                    {
                        // Return the boolean value
                        if let Some(EvaluationResult::Boolean(result, type_info, _)) =
                            param_map.get("valueBoolean")
                        {
                            return Ok(EvaluationResult::Boolean(*result, type_info.clone(), None));
                        }
                    }
                }
            }
        }
    }

    // If we couldn't extract the result, return false
    Ok(EvaluationResult::boolean(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_coding_from_string() {
        let code = EvaluationResult::string("12345".to_string());
        let (system, code_str) = extract_coding(&code).unwrap();
        assert_eq!(system, "");
        assert_eq!(code_str, "12345");
    }

    #[test]
    fn test_extract_coding_from_object() {
        let mut map = HashMap::new();
        map.insert(
            "system".to_string(),
            EvaluationResult::string("http://loinc.org".to_string()),
        );
        map.insert(
            "code".to_string(),
            EvaluationResult::string("1234-5".to_string()),
        );
        map.insert(
            "display".to_string(),
            EvaluationResult::string("Test Code".to_string()),
        );

        let coding = EvaluationResult::Object {
            map,
            type_info: None,
        };

        let (system, code, display) = extract_coding_with_display(&coding).unwrap();
        assert_eq!(system, "http://loinc.org");
        assert_eq!(code, "1234-5");
        assert_eq!(display, Some("Test Code".to_string()));
    }
}
