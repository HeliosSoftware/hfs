//! FHIR version extractor.
//!
//! Extracts FHIR version information from HTTP headers for use in handlers.

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};
use helios_fhir::FhirVersion;

use crate::middleware::content_type::{get_accept_fhir_version, get_content_type_fhir_version};

/// Extractor for FHIR version information from request headers.
///
/// This extractor parses the `fhirVersion` parameter from:
/// - Content-Type header (for writes): `application/fhir+json; fhirVersion=4.0`
/// - Accept header (for reads): `application/fhir+json; fhirVersion=4.0`
///
/// Per the FHIR spec: <https://hl7.org/fhir/http.html#version-parameter>
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::extractors::FhirVersionExtractor;
/// use helios_fhir::FhirVersion;
///
/// async fn handler(version: FhirVersionExtractor) {
///     // Get the version for a write operation
///     let write_version = version.content_version().unwrap_or(FhirVersion::default());
///
///     // Get the version for a read operation
///     if let Some(requested) = version.accept_version() {
///         // Validate that the stored resource matches the requested version
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct FhirVersionExtractor {
    /// Version from Content-Type header (for writes)
    content_version: Option<FhirVersion>,
    /// Version from Accept header (for reads)
    accept_version: Option<FhirVersion>,
}

impl FhirVersionExtractor {
    /// Returns the FHIR version from the Content-Type header.
    ///
    /// This is used for write operations (POST, PUT) to determine
    /// what version the incoming resource conforms to.
    pub fn content_version(&self) -> Option<FhirVersion> {
        self.content_version
    }

    /// Returns the FHIR version from the Accept header.
    ///
    /// This is used for read operations (GET) to determine
    /// what version the client is expecting.
    pub fn accept_version(&self) -> Option<FhirVersion> {
        self.accept_version
    }

    /// Returns the FHIR version to use for storage.
    ///
    /// Uses the Content-Type version if specified, otherwise falls back
    /// to the default FHIR version.
    pub fn storage_version(&self) -> FhirVersion {
        self.content_version
            .unwrap_or_else(helios_fhir::FhirVersion::default_enabled)
    }
}

impl<S> FromRequestParts<S> for FhirVersionExtractor
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let content_version = get_content_type_fhir_version(&parts.headers);
        let accept_version = get_accept_fhir_version(&parts.headers);

        Ok(FhirVersionExtractor {
            content_version,
            accept_version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, header};

    fn create_headers(content_type: Option<&str>, accept: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(ct) = content_type {
            headers.insert(header::CONTENT_TYPE, ct.parse().unwrap());
        }
        if let Some(acc) = accept {
            headers.insert(header::ACCEPT, acc.parse().unwrap());
        }
        headers
    }

    #[test]
    fn test_parse_content_type_version() {
        let headers = create_headers(Some("application/fhir+json; fhirVersion=4.0"), None);
        let version = get_content_type_fhir_version(&headers);
        assert_eq!(version, Some(FhirVersion::default()));
    }

    #[test]
    fn test_parse_accept_version() {
        let headers = create_headers(None, Some("application/fhir+json; fhirVersion=4.0"));
        let version = get_accept_fhir_version(&headers);
        assert_eq!(version, Some(FhirVersion::default()));
    }

    #[test]
    fn test_no_version_returns_none() {
        let headers = create_headers(Some("application/fhir+json"), Some("application/fhir+json"));
        assert_eq!(get_content_type_fhir_version(&headers), None);
        assert_eq!(get_accept_fhir_version(&headers), None);
    }

    #[test]
    fn test_storage_version_default() {
        let extractor = FhirVersionExtractor {
            content_version: None,
            accept_version: None,
        };
        assert_eq!(extractor.storage_version(), FhirVersion::default());
    }
}
