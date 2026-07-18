//! Terminology client for FHIR terminology server operations
//!
//! This module provides an async HTTP client for interacting with FHIR terminology servers.
//! It implements the standard FHIR terminology operations including expand, lookup,
//! validate-code, subsumes, and translate.

use reqwest::Client;
use serde_json::{Value, json};
use std::collections::HashMap;

use crate::error::{FhirPathError, FhirPathResult};
use helios_fhir::FhirVersion;

/// Terminology client for making requests to a FHIR terminology server
#[derive(Clone)]
pub struct TerminologyClient {
    client: Client,
    base_url: String,
    #[allow(dead_code)]
    fhir_version: FhirVersion,
}

impl TerminologyClient {
    /// Creates a new terminology client
    ///
    /// # Arguments
    ///
    /// * `base_url` - The base URL of the terminology server
    /// * `fhir_version` - The FHIR version to use for requests
    pub fn new(base_url: String, fhir_version: FhirVersion) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            fhir_version,
        }
    }

    /// Creates a new terminology client with custom HTTP client
    ///
    /// # Arguments
    ///
    /// * `client` - Custom reqwest client (for authentication, timeouts, etc.)
    /// * `base_url` - The base URL of the terminology server
    /// * `fhir_version` - The FHIR version to use for requests
    #[allow(dead_code)]
    pub fn with_client(client: Client, base_url: String, fhir_version: FhirVersion) -> Self {
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            fhir_version,
        }
    }

    /// Expands a ValueSet
    ///
    /// # Arguments
    ///
    /// * `value_set_url` - URL of the ValueSet to expand
    /// * `params` - Additional parameters for the expansion
    pub async fn expand(
        &self,
        value_set_url: &str,
        params: Option<HashMap<String, String>>,
    ) -> FhirPathResult<Value> {
        let url = format!("{}/ValueSet/$expand", self.base_url);

        // Build query parameters
        let mut query_params = vec![("url".to_string(), value_set_url.to_string())];

        if let Some(params) = params {
            for (key, value) in params {
                query_params.push((key.clone(), value));
            }
        }

        let response = self
            .client
            .get(&url)
            .query(
                &query_params
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect::<Vec<_>>(),
            )
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| FhirPathError::NetworkError(e.to_string()))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|e| FhirPathError::ParseError(e.to_string()))
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(FhirPathError::TerminologyError(format!(
                "ValueSet expansion failed with status {}: {}",
                status, body
            )))
        }
    }

    /// Looks up details for a code
    ///
    /// # Arguments
    ///
    /// * `system` - The code system
    /// * `code` - The code to look up
    /// * `params` - Additional parameters
    pub async fn lookup(
        &self,
        system: &str,
        code: &str,
        params: Option<HashMap<String, String>>,
    ) -> FhirPathResult<Value> {
        let url = format!("{}/CodeSystem/$lookup", self.base_url);

        let mut body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "system",
                    "valueUri": system
                },
                {
                    "name": "code",
                    "valueCode": code
                }
            ]
        });

        // Add additional parameters if provided
        if let Some(params) = params {
            if let Some(parameters) = body.get_mut("parameter").and_then(|p| p.as_array_mut()) {
                for (key, value) in params {
                    parameters.push(json!({
                        "name": key,
                        "valueString": value
                    }));
                }
            }
        }

        let response = self
            .client
            .post(&url)
            .json(&body)
            .header("Content-Type", "application/fhir+json")
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| FhirPathError::NetworkError(e.to_string()))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|e| FhirPathError::ParseError(e.to_string()))
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(FhirPathError::TerminologyError(format!(
                "Code lookup failed with status {}: {}",
                status, body
            )))
        }
    }

    /// Validates a code against a ValueSet
    ///
    /// # Arguments
    ///
    /// * `value_set_url` - URL of the ValueSet
    /// * `system` - The code system
    /// * `code` - The code to validate
    /// * `display` - Optional display text
    /// * `params` - Additional parameters
    pub async fn validate_vs(
        &self,
        value_set_url: &str,
        system: Option<&str>,
        code: &str,
        display: Option<&str>,
        params: Option<HashMap<String, String>>,
    ) -> FhirPathResult<Value> {
        let url = format!("{}/ValueSet/$validate-code", self.base_url);

        let mut parameters = vec![json!({
            "name": "url",
            "valueUri": value_set_url
        })];

        // If we have a system, use coding parameter, otherwise use code parameter
        if let Some(system) = system {
            let mut coding = json!({
                "system": system,
                "code": code
            });
            if let Some(display) = display {
                coding["display"] = json!(display);
            }
            parameters.push(json!({
                "name": "coding",
                "valueCoding": coding
            }));
        } else {
            // For codes without system, use the code parameter
            parameters.push(json!({
                "name": "code",
                "valueCode": code
            }));
            // Tell the server to infer the system from the ValueSet
            parameters.push(json!({
                "name": "inferSystem",
                "valueBoolean": true
            }));
            if let Some(display) = display {
                parameters.push(json!({
                    "name": "display",
                    "valueString": display
                }));
            }
        }

        // Add additional parameters if provided
        if let Some(params) = params {
            for (key, value) in params {
                parameters.push(json!({
                    "name": key,
                    "valueString": value
                }));
            }
        }

        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .header("Content-Type", "application/fhir+json")
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| FhirPathError::NetworkError(e.to_string()))?;

        if response.status().is_success() {
            let result: Value = response
                .json()
                .await
                .map_err(|e| FhirPathError::ParseError(e.to_string()))?;

            Ok(result)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(FhirPathError::TerminologyError(format!(
                "ValueSet validation failed with status {}: {}",
                status, body
            )))
        }
    }

    /// Validates a code against a CodeSystem
    ///
    /// # Arguments
    ///
    /// * `code_system_url` - URL of the CodeSystem
    /// * `code` - The code to validate
    /// * `display` - Optional display text
    /// * `params` - Additional parameters
    pub async fn validate_cs(
        &self,
        code_system_url: &str,
        code: &str,
        display: Option<&str>,
        params: Option<HashMap<String, String>>,
    ) -> FhirPathResult<Value> {
        let url = format!("{}/CodeSystem/$validate-code", self.base_url);

        let mut parameters = vec![
            json!({
                "name": "url",
                "valueUri": code_system_url
            }),
            json!({
                "name": "code",
                "valueCode": code
            }),
        ];

        if let Some(display) = display {
            parameters.push(json!({
                "name": "display",
                "valueString": display
            }));
        }

        // Add additional parameters if provided
        if let Some(params) = params {
            for (key, value) in params {
                parameters.push(json!({
                    "name": key,
                    "valueString": value
                }));
            }
        }

        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .header("Content-Type", "application/fhir+json")
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| FhirPathError::NetworkError(e.to_string()))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|e| FhirPathError::ParseError(e.to_string()))
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(FhirPathError::TerminologyError(format!(
                "CodeSystem validation failed with status {}: {}",
                status, body
            )))
        }
    }

    /// Checks if one code subsumes another
    ///
    /// # Arguments
    ///
    /// * `system` - The code system
    /// * `code_a` - First code
    /// * `code_b` - Second code
    /// * `params` - Additional parameters
    pub async fn subsumes(
        &self,
        system: &str,
        code_a: &str,
        code_b: &str,
        params: Option<HashMap<String, String>>,
    ) -> FhirPathResult<Value> {
        let url = format!("{}/CodeSystem/$subsumes", self.base_url);

        let mut parameters = vec![
            json!({
                "name": "system",
                "valueUri": system
            }),
            json!({
                "name": "codeA",
                "valueCode": code_a
            }),
            json!({
                "name": "codeB",
                "valueCode": code_b
            }),
        ];

        // Add additional parameters if provided
        if let Some(params) = params {
            for (key, value) in params {
                parameters.push(json!({
                    "name": key,
                    "valueString": value
                }));
            }
        }

        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .header("Content-Type", "application/fhir+json")
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| FhirPathError::NetworkError(e.to_string()))?;

        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|e| FhirPathError::ParseError(e.to_string()))
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(FhirPathError::TerminologyError(format!(
                "Subsumes check failed with status {}: {}",
                status, body
            )))
        }
    }

    /// Translates a code using a ConceptMap
    ///
    /// # Arguments
    ///
    /// * `concept_map_url` - URL of the ConceptMap
    /// * `system` - Source system
    /// * `code` - Code to translate
    /// * `target_system` - Optional target system
    /// * `params` - Additional parameters
    pub async fn translate(
        &self,
        concept_map_url: &str,
        system: &str,
        code: &str,
        target_system: Option<&str>,
        params: Option<HashMap<String, String>>,
    ) -> FhirPathResult<Value> {
        let url = format!("{}/ConceptMap/$translate", self.base_url);

        let mut parameters = vec![json!({
            "name": "url",
            "valueUri": concept_map_url
        })];

        // Create a coding parameter
        if !system.is_empty() {
            parameters.push(json!({
                "name": "coding",
                "valueCoding": {
                    "system": system,
                    "code": code
                }
            }));
        } else {
            // For codes without explicit system, we need to infer it from context
            // For FHIR ConceptMaps, certain codes have known systems
            let inferred_system = match code {
                "home" | "work" | "temp" | "old" | "billing" => "http://hl7.org/fhir/address-use",
                "male" | "female" | "other" | "unknown" => {
                    "http://hl7.org/fhir/administrative-gender"
                }
                _ => "",
            };

            if !inferred_system.is_empty() {
                parameters.push(json!({
                    "name": "coding",
                    "valueCoding": {
                        "system": inferred_system,
                        "code": code
                    }
                }));
            } else {
                // Fallback to just code
                parameters.push(json!({
                    "name": "code",
                    "valueCode": code
                }));
            }
        }

        if let Some(target) = target_system {
            parameters.push(json!({
                "name": "targetSystem",
                "valueUri": target
            }));
        }

        // Add additional parameters if provided
        if let Some(params) = params {
            for (key, value) in params {
                parameters.push(json!({
                    "name": key,
                    "valueString": value
                }));
            }
        }

        let body = json!({
            "resourceType": "Parameters",
            "parameter": parameters
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .header("Content-Type", "application/fhir+json")
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| FhirPathError::NetworkError(e.to_string()))?;

        if response.status().is_success() {
            let result: Value = response
                .json()
                .await
                .map_err(|e| FhirPathError::ParseError(e.to_string()))?;

            Ok(result)
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(FhirPathError::TerminologyError(format!(
                "Translation failed with status {}: {}",
                status, body
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminology_client_creation() {
        let client = TerminologyClient::new("https://tx.fhir.org/r4/".to_string(), FhirVersion::R4);
        assert_eq!(client.base_url, "https://tx.fhir.org/r4");
    }

    #[test]
    fn test_base_url_normalization() {
        let client = TerminologyClient::new("https://tx.fhir.org/r4/".to_string(), FhirVersion::R4);
        assert_eq!(client.base_url, "https://tx.fhir.org/r4");

        let client2 = TerminologyClient::new("https://tx.fhir.org/r4".to_string(), FhirVersion::R4);
        assert_eq!(client2.base_url, "https://tx.fhir.org/r4");
    }
}
