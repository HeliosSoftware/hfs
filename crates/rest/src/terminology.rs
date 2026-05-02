//! Terminology service client for HFS/HTS integration.
//!
//! This module provides an async HTTP client for communicating with the
//! Helios Terminology Server (HTS) or any compatible FHIR terminology server.
//! It is used internally by the FHIR search handler to resolve `:in` and `:not-in`
//! search modifiers via `POST /ValueSet/$expand`.
//!
//! # Usage
//!
//! Configure the terminology server via the `HFS_TERMINOLOGY_SERVER` environment
//! variable (or `--terminology-server` CLI flag). When set, the search handler
//! will automatically expand ValueSet references in `:in` and `:not-in` modifiers
//! before querying the storage backend.
//!
//! # Architecture
//!
//! The client is intentionally lightweight — it only implements the subset of FHIR
//! terminology operations required by the REST layer:
//!
//! | Operation | Endpoint | Used by |
//! |-----------|----------|---------|
//! | ValueSet $expand | `POST /ValueSet/$expand` | `:in` / `:not-in` search modifiers |

use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};

/// Errors that can occur when communicating with a terminology server.
#[derive(Debug, thiserror::Error)]
pub enum TerminologyError {
    /// The HTTP request itself failed (network issue, DNS, timeout).
    #[error("Terminology server request failed: {0}")]
    Network(String),

    /// The server responded with a non-2xx status code.
    #[error("Terminology server returned HTTP {status}: {body}")]
    ServerError {
        /// HTTP status code.
        status: u16,
        /// Response body (truncated at 512 bytes for safety).
        body: String,
    },

    /// The response body could not be parsed as expected.
    #[error("Failed to parse terminology server response: {0}")]
    Parse(String),
}

/// A single expanded code from a FHIR ValueSet expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedCode {
    /// Code system URL (e.g. `http://snomed.info/sct`). May be empty.
    pub system: String,
    /// The code value (e.g. `73211009`).
    pub code: String,
    /// Optional human-readable display text.
    pub display: Option<String>,
}

impl ExpandedCode {
    /// Formats the code as `system|code` if a system is present, or just `code`.
    ///
    /// This is the format expected by FHIR token search parameters.
    pub fn as_token(&self) -> String {
        if self.system.is_empty() {
            self.code.clone()
        } else {
            format!("{}|{}", self.system, self.code)
        }
    }
}

/// Async HTTP client for FHIR terminology server operations.
///
/// Used by the HFS REST layer to expand ValueSets when processing `:in` and
/// `:not-in` search modifiers. Targets the Helios Terminology Server (HTS)
/// by default but is compatible with any FHIR R4+ terminology server.
#[derive(Clone)]
pub struct TerminologyServiceClient {
    client: Client,
    /// Base URL of the terminology server (no trailing slash).
    base_url: String,
}

impl TerminologyServiceClient {
    /// Creates a new client targeting the given base URL.
    ///
    /// Trailing slashes in `base_url` are trimmed automatically.
    /// A 10-second timeout is applied to all requests.
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Expands a ValueSet by URL and returns the codes in its expansion.
    ///
    /// Sends `POST {base_url}/ValueSet/$expand` with a FHIR Parameters body
    /// containing the ValueSet URL. Parses the `expansion.contains` array in
    /// the response and returns it as a `Vec<ExpandedCode>`.
    ///
    /// # Errors
    ///
    /// Returns a [`TerminologyError`] if:
    /// - The HTTP request fails (network error, timeout)
    /// - The server returns a non-2xx status
    /// - The response cannot be parsed as a FHIR ValueSet with `expansion.contains`
    pub async fn expand_value_set(
        &self,
        value_set_url: &str,
    ) -> Result<Vec<ExpandedCode>, TerminologyError> {
        let endpoint = format!("{}/ValueSet/$expand", self.base_url);

        let body = json!({
            "resourceType": "Parameters",
            "parameter": [
                {
                    "name": "url",
                    "valueUri": value_set_url
                }
            ]
        });

        let response = self
            .client
            .post(&endpoint)
            .json(&body)
            .header("Content-Type", "application/fhir+json")
            .header("Accept", "application/fhir+json")
            .send()
            .await
            .map_err(|e| TerminologyError::Network(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body_text = response
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(512)
                .collect::<String>();
            return Err(TerminologyError::ServerError {
                status,
                body: body_text,
            });
        }

        let value: Value = response
            .json()
            .await
            .map_err(|e| TerminologyError::Parse(e.to_string()))?;

        extract_expansion_codes(&value)
    }
}

/// Extracts `ExpandedCode` entries from a FHIR ValueSet resource with expansion.
///
/// Expects the standard FHIR structure:
/// ```json
/// { "expansion": { "contains": [{ "system": "...", "code": "...", "display": "..." }] } }
/// ```
fn extract_expansion_codes(value: &Value) -> Result<Vec<ExpandedCode>, TerminologyError> {
    let contains = value
        .pointer("/expansion/contains")
        .and_then(|c| c.as_array())
        .ok_or_else(|| {
            TerminologyError::Parse(
                "ValueSet expansion response is missing /expansion/contains".to_string(),
            )
        })?;

    let mut codes = Vec::with_capacity(contains.len());

    for entry in contains {
        let code = entry
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if code.is_empty() {
            continue;
        }

        let system = entry
            .get("system")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let display = entry
            .get("display")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        codes.push(ExpandedCode {
            system,
            code,
            display,
        });
    }

    Ok(codes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ─── extract_expansion_codes ────────────────────────────────────────────

    #[test]
    fn test_extract_two_codes() {
        let response = json!({
            "resourceType": "ValueSet",
            "expansion": {
                "total": 2,
                "contains": [
                    {"system": "http://example.org/cs", "code": "A", "display": "Alpha"},
                    {"system": "http://example.org/cs", "code": "B", "display": "Beta"}
                ]
            }
        });

        let codes = extract_expansion_codes(&response).unwrap();
        assert_eq!(codes.len(), 2);
        assert_eq!(codes[0].code, "A");
        assert_eq!(codes[0].system, "http://example.org/cs");
        assert_eq!(codes[0].display, Some("Alpha".to_string()));
        assert_eq!(codes[1].code, "B");
        assert_eq!(codes[1].display, Some("Beta".to_string()));
    }

    #[test]
    fn test_extract_empty_expansion() {
        let response = json!({
            "resourceType": "ValueSet",
            "expansion": {
                "total": 0,
                "contains": []
            }
        });

        let codes = extract_expansion_codes(&response).unwrap();
        assert!(codes.is_empty());
    }

    #[test]
    fn test_extract_missing_expansion() {
        let response = json!({"resourceType": "ValueSet"});
        let result = extract_expansion_codes(&response);
        assert!(matches!(result, Err(TerminologyError::Parse(_))));
    }

    #[test]
    fn test_extract_skips_empty_codes() {
        let response = json!({
            "expansion": {
                "contains": [
                    {"system": "http://example.org/cs", "code": ""},
                    {"system": "http://example.org/cs", "code": "X"}
                ]
            }
        });

        let codes = extract_expansion_codes(&response).unwrap();
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].code, "X");
    }

    #[test]
    fn test_extract_no_system() {
        let response = json!({
            "expansion": {
                "contains": [{"code": "Z"}]
            }
        });

        let codes = extract_expansion_codes(&response).unwrap();
        assert_eq!(codes.len(), 1);
        assert_eq!(codes[0].code, "Z");
        assert_eq!(codes[0].system, "");
    }

    // ─── ExpandedCode::as_token ─────────────────────────────────────────────

    #[test]
    fn test_as_token_with_system() {
        let c = ExpandedCode {
            system: "http://snomed.info/sct".to_string(),
            code: "73211009".to_string(),
            display: None,
        };
        assert_eq!(c.as_token(), "http://snomed.info/sct|73211009");
    }

    #[test]
    fn test_as_token_without_system() {
        let c = ExpandedCode {
            system: String::new(),
            code: "active".to_string(),
            display: None,
        };
        assert_eq!(c.as_token(), "active");
    }

    // ─── TerminologyServiceClient ────────────────────────────────────────────

    #[test]
    fn test_client_trims_trailing_slash() {
        let client = TerminologyServiceClient::new("http://localhost:8090/".to_string());
        assert_eq!(client.base_url(), "http://localhost:8090");
    }

    #[test]
    fn test_client_no_trailing_slash() {
        let client = TerminologyServiceClient::new("http://localhost:8090".to_string());
        assert_eq!(client.base_url(), "http://localhost:8090");
    }
}
