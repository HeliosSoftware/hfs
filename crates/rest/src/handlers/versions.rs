//! FHIR $versions operation handler.
//!
//! Implements the `$versions` operation as defined in the FHIR specification:
//! <https://build.fhir.org/capabilitystatement-operation-versions.html>
//!
//! This operation returns a Parameters resource listing all supported FHIR versions.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use helios_fhir::FhirVersion;
use helios_persistence::core::ResourceStorage;
use tracing::debug;

use crate::error::RestResult;
use crate::state::AppState;

/// Handler for the $versions operation.
///
/// Returns a Parameters resource containing all supported FHIR versions.
///
/// # HTTP Request
///
/// `GET [base]/$versions`
///
/// # Response
///
/// Returns a Parameters resource (200 OK) with:
/// - `version` parameters for each supported version
/// - `default` parameter indicating the default version
///
/// # Example Response
///
/// ```json
/// {
///   "resourceType": "Parameters",
///   "parameter": [
///     { "name": "version", "valueCode": "4.0" },
///     { "name": "version", "valueCode": "5.0" },
///     { "name": "default", "valueCode": "4.0" }
///   ]
/// }
/// ```
pub async fn versions_handler<S>(State(_state): State<AppState<S>>) -> RestResult<Response>
where
    S: ResourceStorage + Send + Sync,
{
    debug!("Processing $versions request");

    // Get all enabled versions
    let enabled_versions = FhirVersion::enabled_versions();
    let default_version = FhirVersion::default_enabled();

    // Build version parameters
    let mut parameters: Vec<serde_json::Value> = enabled_versions
        .iter()
        .map(|v| {
            serde_json::json!({
                "name": "version",
                "valueCode": v.as_mime_param()
            })
        })
        .collect();

    // Add default version parameter
    parameters.push(serde_json::json!({
        "name": "default",
        "valueCode": default_version.as_mime_param()
    }));

    let response = serde_json::json!({
        "resourceType": "Parameters",
        "parameter": parameters
    });

    Ok((StatusCode::OK, Json(response)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enabled_versions_not_empty() {
        let versions = FhirVersion::enabled_versions();
        assert!(
            !versions.is_empty(),
            "At least one FHIR version should be enabled"
        );
    }
}
