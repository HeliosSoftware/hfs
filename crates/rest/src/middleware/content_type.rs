//! Content negotiation middleware.
//!
//! Handles content type negotiation for FHIR requests and responses.

use axum::http::{HeaderMap, StatusCode, header};
use helios_fhir::FhirVersion;

/// Supported FHIR content types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FhirFormat {
    /// JSON format (application/fhir+json)
    Json,
    /// XML format (application/fhir+xml)
    Xml,
    /// NDJSON format (application/fhir+ndjson) - for bulk operations
    NdJson,
}

/// Parsed FHIR content type with optional version parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FhirContentType {
    /// The format (json, xml, ndjson)
    pub format: FhirFormat,
    /// The FHIR version parameter, if specified (e.g., from `fhirVersion=4.0`)
    pub fhir_version: Option<FhirVersion>,
}

impl FhirContentType {
    /// Creates a new FhirContentType with the given format and no version.
    pub fn new(format: FhirFormat) -> Self {
        Self {
            format,
            fhir_version: None,
        }
    }

    /// Creates a new FhirContentType with the given format and version.
    pub fn with_version(format: FhirFormat, version: FhirVersion) -> Self {
        Self {
            format,
            fhir_version: Some(version),
        }
    }

    /// Returns the MIME type string for this content type (without version parameter).
    pub fn mime_type(&self) -> &'static str {
        self.format.mime_type()
    }

    /// Returns the full Content-Type header value, including fhirVersion if present.
    pub fn to_header_value(&self) -> String {
        match self.fhir_version {
            Some(version) => format!(
                "{}; fhirVersion={}",
                self.format.mime_type(),
                version.as_mime_param()
            ),
            None => self.format.mime_type().to_string(),
        }
    }

    /// Parses a content type string with optional fhirVersion parameter.
    ///
    /// Example: "application/fhir+json; fhirVersion=4.0"
    pub fn parse(content_type: &str) -> Option<Self> {
        // Split on semicolon to separate media type from parameters
        let parts: Vec<&str> = content_type.split(';').map(|s| s.trim()).collect();

        // Parse the media type
        let format = FhirFormat::parse(parts.first()?)?;

        // Parse optional fhirVersion parameter
        let fhir_version = parts.iter().skip(1).find_map(|param| {
            let param_parts: Vec<&str> = param.splitn(2, '=').map(|s| s.trim()).collect();
            if param_parts.len() == 2 && param_parts[0].eq_ignore_ascii_case("fhirVersion") {
                FhirVersion::from_mime_param(param_parts[1])
            } else {
                None
            }
        });

        Some(Self {
            format,
            fhir_version,
        })
    }
}

/// Supported FHIR content types.
/// Kept for backwards compatibility - use FhirContentType for new code.
pub type FhirContentFormat = FhirFormat;

impl FhirFormat {
    /// Returns the MIME type string for this format.
    pub fn mime_type(&self) -> &'static str {
        match self {
            FhirFormat::Json => "application/fhir+json",
            FhirFormat::Xml => "application/fhir+xml",
            FhirFormat::NdJson => "application/fhir+ndjson",
        }
    }

    /// Parses a media type string into a FhirFormat.
    pub fn parse(media_type: &str) -> Option<Self> {
        let ct = media_type.to_lowercase();

        if ct.contains("fhir+json") || ct.contains("application/json") {
            Some(FhirFormat::Json)
        } else if ct.contains("fhir+xml") || ct.contains("application/xml") {
            Some(FhirFormat::Xml)
        } else if ct.contains("fhir+ndjson") || ct.contains("application/ndjson") {
            Some(FhirFormat::NdJson)
        } else {
            None
        }
    }
}

/// Determines the response format from `_format` query parameter and Accept header.
///
/// Follows FHIR spec precedence: `_format` > Accept header > default JSON.
///
/// Supported `_format` values:
/// - `json`, `application/json`, `application/fhir+json`
/// - `xml`, `application/xml`, `application/fhir+xml`
/// - `ndjson`, `application/ndjson`, `application/fhir+ndjson`
pub fn negotiate_format(headers: &HeaderMap, format_param: Option<&str>) -> FhirContentType {
    // _format takes precedence per FHIR spec
    if let Some(format) = format_param {
        let resolved = match format.to_lowercase().as_str() {
            "json" | "application/json" | "application/fhir+json" => Some(FhirFormat::Json),
            "xml" | "application/xml" | "application/fhir+xml" => Some(FhirFormat::Xml),
            "ndjson" | "application/ndjson" | "application/fhir+ndjson" => Some(FhirFormat::NdJson),
            _ => FhirFormat::parse(format),
        };
        if let Some(fmt) = resolved {
            return FhirContentType::new(fmt);
        }
    }

    // Fall back to Accept header
    negotiate_content_type(headers)
}

/// Determines the response content type from the Accept header.
///
/// Returns JSON if no Accept header is present or if the client accepts
/// multiple types including JSON.
///
/// Per the FHIR spec, a wildcard Accept type (`*/*` or `application/*`) means
/// "server's default", which is JSON. Critically, if a wildcard is present
/// alongside generic types like `application/xml` (as browsers send), the
/// wildcard takes precedence and JSON is returned — only an explicit
/// FHIR-specific type (`application/fhir+json`, `application/fhir+xml`) overrides this.
pub fn negotiate_content_type(headers: &HeaderMap) -> FhirContentType {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/fhir+json");

    let entries: Vec<&str> = accept.split(',').map(|s| s.trim()).collect();

    // If a wildcard is present and no explicit FHIR-specific type is requested,
    // return JSON. This handles browsers (which send application/xml alongside */*),
    // curl (which sends */*), and any other client that hasn't specifically asked
    // for a FHIR format. Only application/fhir+json and application/fhir+xml are
    // considered "explicit FHIR types" — generic aliases like application/xml are not.
    let has_wildcard = entries.iter().any(|e| {
        let mt = e.split(';').next().unwrap_or("").trim();
        mt == "*/*" || mt == "application/*"
    });
    let has_explicit_fhir_type = entries.iter().any(|e| {
        let mt = e.split(';').next().unwrap_or("").trim().to_lowercase();
        mt.contains("fhir+json") || mt.contains("fhir+xml") || mt.contains("fhir+ndjson")
    });
    if has_wildcard && !has_explicit_fhir_type {
        return FhirContentType::new(FhirFormat::Json);
    }

    // No wildcard present (or explicit FHIR type overrides it): match in order
    for entry in &entries {
        let media_type = entry.split(';').next().unwrap_or("").trim();

        if let Some(ct) = FhirContentType::parse(entry) {
            return ct;
        }

        if media_type == "*/*" || media_type == "application/*" {
            return FhirContentType::new(FhirFormat::Json);
        }
    }

    // Default to JSON if nothing matched
    FhirContentType::new(FhirFormat::Json)
}

/// Validates the Content-Type header for incoming requests.
///
/// Returns an error status if the content type is not supported.
pub fn validate_request_content_type(headers: &HeaderMap) -> Result<FhirContentType, StatusCode> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());

    match content_type {
        Some(ct) => FhirContentType::parse(ct).ok_or(StatusCode::UNSUPPORTED_MEDIA_TYPE),
        None => {
            // If no Content-Type, assume JSON with default version
            Ok(FhirContentType::new(FhirFormat::Json))
        }
    }
}

/// Extracts the fhirVersion from the Accept header, if present.
pub fn get_accept_fhir_version(headers: &HeaderMap) -> Option<FhirVersion> {
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok())?;

    // Parse Accept header and extract fhirVersion
    for media_type in accept.split(',') {
        if let Some(ct) = FhirContentType::parse(media_type.trim()) {
            if ct.fhir_version.is_some() {
                return ct.fhir_version;
            }
        }
    }
    None
}

/// Extracts the fhirVersion from the Content-Type header, if present.
pub fn get_content_type_fhir_version(headers: &HeaderMap) -> Option<FhirVersion> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())?;

    FhirContentType::parse(content_type).and_then(|ct| ct.fhir_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_format() {
        assert_eq!(
            FhirFormat::parse("application/fhir+json"),
            Some(FhirFormat::Json)
        );
        assert_eq!(
            FhirFormat::parse("application/fhir+xml"),
            Some(FhirFormat::Xml)
        );
        assert_eq!(
            FhirFormat::parse("application/json"),
            Some(FhirFormat::Json)
        );
        assert_eq!(FhirFormat::parse("text/plain"), None);
    }

    #[test]
    fn test_parse_content_type_simple() {
        let ct = FhirContentType::parse("application/fhir+json").unwrap();
        assert_eq!(ct.format, FhirFormat::Json);
        assert_eq!(ct.fhir_version, None);
    }

    #[test]
    fn test_parse_content_type_with_version() {
        let ct = FhirContentType::parse("application/fhir+json; fhirVersion=4.0").unwrap();
        assert_eq!(ct.format, FhirFormat::Json);
        assert_eq!(ct.fhir_version, Some(FhirVersion::default()));

        #[cfg(feature = "R5")]
        {
            let ct5 = FhirContentType::parse("application/fhir+json; fhirVersion=5.0").unwrap();
            assert_eq!(ct5.fhir_version, Some(FhirVersion::R5));
        }
    }

    #[test]
    fn test_to_header_value() {
        let ct = FhirContentType::new(FhirFormat::Json);
        assert_eq!(ct.to_header_value(), "application/fhir+json");

        let ct_with_version =
            FhirContentType::with_version(FhirFormat::Json, FhirVersion::default());
        assert!(ct_with_version.to_header_value().contains("fhirVersion="));
    }

    #[test]
    fn test_mime_type() {
        assert_eq!(FhirFormat::Json.mime_type(), "application/fhir+json");
        assert_eq!(FhirFormat::Xml.mime_type(), "application/fhir+xml");
    }

    fn make_headers(accept: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, accept.parse().unwrap());
        headers
    }

    #[test]
    fn test_negotiate_browser_accept_returns_json() {
        // Browsers send application/xml alongside */* — should default to JSON
        let headers =
            make_headers("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8");
        let ct = negotiate_content_type(&headers);
        assert_eq!(ct.format, FhirFormat::Json);
    }

    #[test]
    fn test_negotiate_wildcard_returns_json() {
        let headers = make_headers("*/*");
        let ct = negotiate_content_type(&headers);
        assert_eq!(ct.format, FhirFormat::Json);
    }

    #[test]
    fn test_negotiate_explicit_fhir_xml_returns_xml() {
        let headers = make_headers("application/fhir+xml");
        let ct = negotiate_content_type(&headers);
        assert_eq!(ct.format, FhirFormat::Xml);
    }

    #[test]
    fn test_negotiate_explicit_fhir_xml_with_wildcard_returns_xml() {
        // Explicit FHIR type overrides wildcard
        let headers = make_headers("application/fhir+xml, */*");
        let ct = negotiate_content_type(&headers);
        assert_eq!(ct.format, FhirFormat::Xml);
    }

    #[test]
    fn test_negotiate_application_xml_only_returns_xml() {
        // application/xml alone (no wildcard) → XML
        let headers = make_headers("application/xml");
        let ct = negotiate_content_type(&headers);
        assert_eq!(ct.format, FhirFormat::Xml);
    }

    #[test]
    fn test_negotiate_no_accept_returns_json() {
        let ct = negotiate_content_type(&HeaderMap::new());
        assert_eq!(ct.format, FhirFormat::Json);
    }
}
