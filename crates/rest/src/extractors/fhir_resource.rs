//! FHIR resource extractor.
//!
//! Extracts and validates FHIR resources from request bodies.

use axum::{
    body::Bytes,
    extract::{FromRequest, Request},
    http::header,
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::error::RestError;
use crate::fhir_types::is_valid_resource_type;

/// Axum extractor for FHIR resources.
///
/// Extracts a FHIR resource from the request body, validating that
/// it has a resourceType field.
///
/// # Example
///
/// ```rust,ignore
/// use helios_rest::extractors::FhirResource;
///
/// async fn create_handler(FhirResource(resource): FhirResource) {
///     println!("Resource type: {}", resource["resourceType"]);
/// }
/// ```
#[derive(Debug)]
pub struct FhirResource(pub Value);

impl FhirResource {
    /// Returns the resource type.
    pub fn resource_type(&self) -> Option<&str> {
        self.0.get("resourceType").and_then(|v| v.as_str())
    }

    /// Returns the resource ID if present.
    pub fn id(&self) -> Option<&str> {
        self.0.get("id").and_then(|v| v.as_str())
    }

    /// Consumes the extractor and returns the inner Value.
    pub fn into_inner(self) -> Value {
        self.0
    }

    /// Returns a reference to the inner Value.
    pub fn inner(&self) -> &Value {
        &self.0
    }
}

/// Error type for FHIR resource extraction failures.
#[derive(Debug)]
pub enum FhirResourceRejection {
    /// JSON parsing failed.
    InvalidJson(String),
    /// Request body exceeds the configured size limit.
    PayloadTooLarge(String),
    /// Missing resourceType field.
    MissingResourceType,
    /// Unsupported content type.
    UnsupportedMediaType(String),
    /// Invalid or unknown resource type.
    InvalidResourceType(String),
}

impl IntoResponse for FhirResourceRejection {
    fn into_response(self) -> Response {
        let error = match self {
            FhirResourceRejection::InvalidJson(msg) => RestError::BadRequest {
                message: format!("Invalid JSON: {}", msg),
            },
            FhirResourceRejection::PayloadTooLarge(msg) => {
                RestError::PayloadTooLarge { message: msg }
            }
            FhirResourceRejection::MissingResourceType => RestError::BadRequest {
                message: "Resource must contain resourceType".to_string(),
            },
            FhirResourceRejection::UnsupportedMediaType(ct) => {
                RestError::UnsupportedMediaType { content_type: ct }
            }
            FhirResourceRejection::InvalidResourceType(rt) => RestError::BadRequest {
                message: format!("Unknown or unsupported resource type: {}", rt),
            },
        };
        error.into_response()
    }
}

impl<S> FromRequest<S> for FhirResource
where
    S: Send + Sync,
{
    type Rejection = FhirResourceRejection;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        // Check content type (must own the string before moving req)
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json")
            .to_string();

        // Extract body bytes. A length-limit rejection must keep its 413
        // status instead of collapsing into a generic 400 "invalid JSON".
        let bytes = Bytes::from_request(req, state).await.map_err(|e| {
            if e.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                FhirResourceRejection::PayloadTooLarge(e.body_text())
            } else {
                FhirResourceRejection::InvalidJson(e.body_text())
            }
        })?;

        // Parse body based on content type
        let value: Value = if content_type.contains("xml") {
            #[cfg(feature = "xml")]
            {
                let xml_str = std::str::from_utf8(&bytes)
                    .map_err(|e| FhirResourceRejection::InvalidJson(e.to_string()))?;
                crate::responses::format::xml_to_value(xml_str)
                    .map_err(FhirResourceRejection::InvalidJson)?
            }
            #[cfg(not(feature = "xml"))]
            {
                return Err(FhirResourceRejection::UnsupportedMediaType(
                    content_type.to_string(),
                ));
            }
        } else if content_type.contains("json") || content_type == "application/json" {
            serde_json::from_slice(&bytes)
                .map_err(|e| FhirResourceRejection::InvalidJson(e.to_string()))?
        } else {
            return Err(FhirResourceRejection::UnsupportedMediaType(
                content_type.to_string(),
            ));
        };

        // Validate resourceType is present
        let resource_type = value
            .get("resourceType")
            .and_then(|v| v.as_str())
            .ok_or(FhirResourceRejection::MissingResourceType)?;

        // Validate resource type is known
        if !is_valid_resource_type(resource_type) {
            return Err(FhirResourceRejection::InvalidResourceType(
                resource_type.to_string(),
            ));
        }

        Ok(FhirResource(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_type() {
        let resource = FhirResource(serde_json::json!({
            "resourceType": "Patient",
            "id": "123"
        }));

        assert_eq!(resource.resource_type(), Some("Patient"));
        assert_eq!(resource.id(), Some("123"));
    }

    #[test]
    fn test_into_inner() {
        let value = serde_json::json!({"resourceType": "Patient"});
        let resource = FhirResource(value.clone());
        assert_eq!(resource.into_inner(), value);
    }
}
