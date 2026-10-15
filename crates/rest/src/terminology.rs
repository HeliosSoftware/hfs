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
//! | ValueSet $expand (inline resource) | `POST /ValueSet/$expand` | SQL Query terminology materialization (#1824) |
//! | ValueSet / ConceptMap read by canonical | `GET /ValueSet?url=`, `GET /ConceptMap?url=` | definitions HFS does not store (#1824, D4) |

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

    /// The response body, or the expansion it carries, is bigger than the
    /// caller's cap. The body is abandoned before it is parsed.
    #[error("Terminology server response exceeds {max} {unit}")]
    TooLarge {
        /// The cap that was exceeded.
        max: usize,
        /// `bytes` or `members`.
        unit: &'static str,
    },

    /// The server answered with a page, not a whole expansion, even after one
    /// bounded re-request.
    #[error("Terminology server returned an incomplete expansion: {0}")]
    Incomplete(String),
}

/// How [`TerminologyServiceClient::expand_value_set_resource`] bounds one
/// expansion (#1824, D9).
#[derive(Debug, Clone, Copy)]
pub struct ExpandOptions {
    /// Per-request timeout, derived from the write's remaining budget.
    pub timeout: Duration,
    /// Most members the expansion may have; a larger `total` is refused
    /// before anything else.
    pub max_members: usize,
    /// Most bytes the response body may be; the body is abandoned, unparsed,
    /// once exceeded.
    pub max_artifact_bytes: usize,
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
    /// A 10-second timeout is applied to all requests, with a short connect
    /// timeout so an unreachable server fails fast — the `:in` fail-open
    /// semantics depend on it.
    ///
    /// The client bypasses any proxy environment: the terminology server is a
    /// configured internal endpoint, and routing it through an egress proxy
    /// turns "connection refused in milliseconds" into a multi-minute hang on
    /// runners whose `no_proxy` uses CIDR ranges reqwest cannot parse. The
    /// previous silent `unwrap_or_default()` fallback could also discard the
    /// timeouts entirely; a client build failure is a startup-visible
    /// misconfiguration, not something to paper over.
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(2))
            .no_proxy()
            .build()
            .expect("terminology HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Like [`Self::new`], with the overall request timeout the caller
    /// chooses. The write-path materializer (#1824) pairs this with its own
    /// budget; `:in` search keeps [`Self::new`]'s 10 s.
    pub fn with_timeout(base_url: String, timeout: Duration) -> Self {
        let client = Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(2).min(timeout))
            .no_proxy()
            .build()
            .expect("terminology HTTP client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Expands a whole ValueSet resource (#1824): `POST {base}/ValueSet/$expand`
    /// with the resource inline as the `valueSet` parameter, so the server
    /// expands **the definition HFS stores** (storage-first precedence) and
    /// the call works whether or not the server knows the URL.
    ///
    /// The response body is read as a stream and abandoned once
    /// `opts.max_artifact_bytes` is exceeded, before any JSON parsing.
    /// Completeness: an expansion carrying `offset` is a page and is refused;
    /// a `total` above `opts.max_members` is refused before anything else; a
    /// `total` above the number of `contains` entries at all depths triggers
    /// **one** re-request with `count = total`, and an expansion that is still
    /// short is refused. Returns the full ValueSet the server answered with.
    pub async fn expand_value_set_resource(
        &self,
        value_set: &Value,
        opts: &ExpandOptions,
    ) -> Result<Value, TerminologyError> {
        let endpoint = format!("{}/ValueSet/$expand", self.base_url);
        let request = |count: Option<usize>| {
            let mut parameter = vec![json!({"name": "valueSet", "resource": value_set})];
            if let Some(count) = count {
                parameter.push(json!({"name": "count", "valueInteger": count}));
            }
            json!({"resourceType": "Parameters", "parameter": parameter})
        };

        let first = self
            .post_bounded(
                &endpoint,
                &request(None),
                opts.timeout,
                opts.max_artifact_bytes,
            )
            .await?;
        match check_expansion_complete(&first, opts.max_members)? {
            Completeness::Complete => Ok(first),
            Completeness::Short { total } => {
                let again = self
                    .post_bounded(
                        &endpoint,
                        &request(Some(total)),
                        opts.timeout,
                        opts.max_artifact_bytes,
                    )
                    .await?;
                match check_expansion_complete(&again, opts.max_members)? {
                    Completeness::Complete => Ok(again),
                    Completeness::Short { total } => Err(TerminologyError::Incomplete(format!(
                        "expansion.total={total} but fewer contains entries were returned even \
                         with count={total}"
                    ))),
                }
            }
        }
    }

    /// `GET {base}/ValueSet?url=...[&version=...]`: the newest match, or
    /// `None` (#1824, D4). The body is bounded by `max_bytes`.
    pub async fn read_value_set_by_canonical(
        &self,
        url: &str,
        version: Option<&str>,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<Option<Value>, TerminologyError> {
        self.read_by_canonical("ValueSet", url, version, timeout, max_bytes)
            .await
    }

    /// `GET {base}/ConceptMap?url=...[&version=...]`: the newest match, or
    /// `None` (#1824, D4). The body is bounded by `max_bytes`.
    pub async fn read_concept_map_by_canonical(
        &self,
        url: &str,
        version: Option<&str>,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<Option<Value>, TerminologyError> {
        self.read_by_canonical("ConceptMap", url, version, timeout, max_bytes)
            .await
    }

    async fn read_by_canonical(
        &self,
        resource_type: &str,
        url: &str,
        version: Option<&str>,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<Option<Value>, TerminologyError> {
        let endpoint = format!("{}/{resource_type}", self.base_url);
        let mut query: Vec<(&str, &str)> = vec![("url", url)];
        if let Some(v) = version {
            query.push(("version", v));
        }
        let response = self
            .client
            .get(&endpoint)
            .query(&query)
            .header("Accept", "application/fhir+json")
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| TerminologyError::Network(e.to_string()))?;
        let bundle = read_bounded_json(response, max_bytes).await?;
        let entries = bundle
            .get("entry")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut best: Option<(String, Value)> = None;
        for entry in entries {
            let Some(resource) = entry.get("resource") else {
                continue;
            };
            if resource.get("resourceType").and_then(Value::as_str) != Some(resource_type)
                || resource.get("url").and_then(Value::as_str) != Some(url)
                || version
                    .is_some_and(|v| resource.get("version").and_then(Value::as_str) != Some(v))
            {
                continue;
            }
            let stamp = resource
                .pointer("/meta/lastUpdated")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if best.as_ref().is_none_or(|(seen, _)| stamp >= *seen) {
                best = Some((stamp, resource.clone()));
            }
        }
        Ok(best.map(|(_, resource)| resource))
    }

    async fn post_bounded(
        &self,
        endpoint: &str,
        body: &Value,
        timeout: Duration,
        max_bytes: usize,
    ) -> Result<Value, TerminologyError> {
        let response = self
            .client
            .post(endpoint)
            .json(body)
            .header("Content-Type", "application/fhir+json")
            .header("Accept", "application/fhir+json")
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| TerminologyError::Network(e.to_string()))?;
        read_bounded_json(response, max_bytes).await
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

    /// Expands the concepts subsumed by (or subsuming) a code, via an inline
    /// ValueSet `$expand` with a filter.
    ///
    /// - `op = "is-a"` returns the code and all its descendants (for `:below`).
    /// - `op = "generalizes"` returns the code and all its ancestors (for `:above`).
    ///
    /// Both include the code itself, per FHIR subsumption-testing semantics.
    pub async fn expand_subsumption(
        &self,
        system: &str,
        code: &str,
        op: &str,
    ) -> Result<Vec<ExpandedCode>, TerminologyError> {
        let endpoint = format!("{}/ValueSet/$expand", self.base_url);

        let body = json!({
            "resourceType": "Parameters",
            "parameter": [{
                "name": "valueSet",
                "resource": {
                    "resourceType": "ValueSet",
                    "compose": {
                        "include": [{
                            "system": system,
                            "filter": [{ "property": "concept", "op": op, "value": code }]
                        }]
                    }
                }
            }]
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

/// Reads a response body chunk by chunk, abandoning it once `max_bytes` is
/// exceeded, then parses it. A non-2xx status is reported with up to 512
/// bytes of the body, as the older methods do.
async fn read_bounded_json(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Value, TerminologyError> {
    let status = response.status();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| TerminologyError::Network(e.to_string()))?
    {
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            if !status.is_success() {
                break;
            }
            return Err(TerminologyError::TooLarge {
                max: max_bytes,
                unit: "bytes",
            });
        }
        buf.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let body = String::from_utf8_lossy(&buf).chars().take(512).collect();
        return Err(TerminologyError::ServerError {
            status: status.as_u16(),
            body,
        });
    }
    serde_json::from_slice(&buf).map_err(|e| TerminologyError::Parse(e.to_string()))
}

enum Completeness {
    Complete,
    Short { total: usize },
}

/// term-24 against a server's answer: no `offset`; `total` within
/// `max_members`; `total` not above the `contains` entries at all depths.
fn check_expansion_complete(
    value_set: &Value,
    max_members: usize,
) -> Result<Completeness, TerminologyError> {
    let expansion = value_set.get("expansion").ok_or_else(|| {
        TerminologyError::Parse("ValueSet expansion response is missing /expansion".to_string())
    })?;
    if let Some(offset) = expansion.get("offset").and_then(Value::as_u64)
        && offset > 0
    {
        return Err(TerminologyError::Incomplete(format!(
            "expansion carries offset={offset}"
        )));
    }
    let total = expansion
        .get("total")
        .and_then(Value::as_u64)
        .map(|t| t as usize);
    if let Some(total) = total
        && total > max_members
    {
        return Err(TerminologyError::TooLarge {
            max: max_members,
            unit: "members",
        });
    }
    let entries =
        count_contains_entries(expansion, max_members.saturating_mul(4).saturating_add(1));
    if entries > max_members.saturating_mul(4) {
        return Err(TerminologyError::TooLarge {
            max: max_members,
            unit: "members",
        });
    }
    match total {
        Some(total) if total > entries => Ok(Completeness::Short { total }),
        _ => Ok(Completeness::Complete),
    }
}

/// Counts `contains` entries at every depth, stopping at `cap`.
fn count_contains_entries(expansion: &Value, cap: usize) -> usize {
    let mut count = 0usize;
    let mut stack: Vec<&Value> = Vec::new();
    if let Some(top) = expansion.get("contains").and_then(Value::as_array) {
        stack.extend(top.iter());
    }
    while let Some(entry) = stack.pop() {
        count += 1;
        if count >= cap {
            return count;
        }
        if let Some(children) = entry.get("contains").and_then(Value::as_array) {
            stack.extend(children.iter());
        }
    }
    count
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

    // ─── expansion completeness (#1824) ──────────────────────────────────────

    #[test]
    fn complete_expansion_counts_nested_entries_against_total() {
        let vs = json!({"expansion": {"total": 3, "contains": [
            {"abstract": true, "contains": [{"code": "a"}, {"code": "b"}]}
        ]}});
        assert!(matches!(
            check_expansion_complete(&vs, 100),
            Ok(Completeness::Complete)
        ));
    }

    #[test]
    fn short_expansion_asks_for_the_total_and_offset_is_refused() {
        let vs = json!({"expansion": {"total": 5, "contains": [{"code": "a"}]}});
        assert!(matches!(
            check_expansion_complete(&vs, 100),
            Ok(Completeness::Short { total: 5 })
        ));
        let paged = json!({"expansion": {"offset": 2, "contains": [{"code": "a"}]}});
        assert!(matches!(
            check_expansion_complete(&paged, 100),
            Err(TerminologyError::Incomplete(_))
        ));
    }

    #[test]
    fn total_over_the_member_cap_is_too_large_before_anything_else() {
        let vs = json!({"expansion": {"total": 1_000, "contains": []}});
        assert!(matches!(
            check_expansion_complete(&vs, 10),
            Err(TerminologyError::TooLarge {
                max: 10,
                unit: "members"
            })
        ));
    }

    #[test]
    fn with_timeout_trims_the_base_url_too() {
        let client = TerminologyServiceClient::with_timeout(
            "http://localhost:8090/".to_string(),
            Duration::from_secs(3),
        );
        assert_eq!(client.base_url(), "http://localhost:8090");
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
