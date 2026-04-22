//! Validation-facing terminology service interfaces and adapters.
//!
//! This module defines the async and sync terminology service contracts used by
//! the validation engine, along with the remote service implementation that
//! adapts simple membership queries into typed backend requests and converts
//! backend `$validate-code` responses into validation-friendly outcomes.
//!
//! For sync validation without a remote server, see [`super::LocalTerminologyService`], which
//! delegates to generated ValueSet helpers in `helios_fhir`.
use crate::ValidationError;
use crate::backend::TerminologyBackend;
use crate::error::{remote_terminology_error_kind_label, validation_error_kind_label};
use crate::helpers::parse_validate_vs_result;
use crate::requests::ValidateVsRequest;
use crate::terminology::helios::HeliosTerminologyBackend;
use crate::terminology::types::TerminologyMembershipOutcome;
use async_trait::async_trait;
use helios_fhir::FhirVersion;
use std::sync::Arc;
use tracing::{debug, instrument, warn};

/// Trait representing a terminology validation service.
///
/// This allows the validator to remain independent of the specific
/// terminology backend (Snowstorm, HAPI, local cache, etc.).
///
/// # Narrow `member_of` contract
///
/// [`TerminologyService::member_of`] is intentionally minimal: it models **one**
/// `(system, code, display)` check, which matches how generated binding validation
/// decomposes [`Coding`](https://hl7.org/fhir/datatypes.html#Coding) and
/// [`CodeableConcept`](https://hl7.org/fhir/datatypes.html#CodeableConcept) for remote
/// follow-up. It does **not** send a full FHIR [`ValidateVsRequest`] (no `coding` /
/// `codeableConcept` JSON, `systemVersion`, `context`, `date`, etc.).
///
/// For a full `$validate-code` request, build a [`ValidateVsRequest`] and call
/// [`RemoteTerminologyService::validate_vs`] (or use [`TerminologyBackend::validate_vs`]
/// directly).
#[async_trait]
pub trait TerminologyService: Send + Sync {
    /// Returns whether the given code is in the value set (narrow `code` / `system` / `display` shape).
    async fn member_of(
        &self,
        valueset_url: &str,
        system: Option<&str>,
        code: &str,
        display: Option<&str>,
    ) -> Result<TerminologyMembershipOutcome, ValidationError>;
}

/// Synchronous terminology validation interface used by backward-compatible
/// sync validation paths.
///
/// This trait is intended for local-only or explicitly synchronous terminology
/// integrations. Production remote terminology lookups should generally prefer
/// the asynchronous [`TerminologyService`] path.
pub trait TerminologyServiceSync {
    fn member_of(
        &self,
        valueset_url: &str,
        system: Option<&str>,
        code: &str,
        display: Option<&str>,
    ) -> Result<TerminologyMembershipOutcome, ValidationError>;
}

/// Terminology service backed by a remote FHIR terminology backend.
///
/// This validation-facing adapter delegates terminology operations to a broader
/// backend abstraction and converts `$validate-code` responses into
/// `TerminologyMembershipOutcome`.
pub struct RemoteTerminologyService {
    backend: Arc<dyn TerminologyBackend>,
}

impl RemoteTerminologyService {
    /// Create a remote terminology service with the default HTTP client.
    ///
    /// This is suitable for local development and simple integration setups.
    /// For production use, prefer `with_client(...)` so timeouts, connection
    /// pooling, authentication, and other transport behavior can be configured
    /// explicitly.
    pub fn new(base_url: String, fhir_version: FhirVersion) -> Self {
        Self {
            backend: Arc::new(HeliosTerminologyBackend::new(base_url, fhir_version)),
        }
    }

    /// Create a remote terminology service with a caller-provided HTTP client.
    ///
    /// This allows production callers to configure request timeout, connect
    /// timeout, connection pooling, keepalive, authentication, proxies, and
    /// related transport settings once and reuse them across terminology calls.
    pub fn with_client(
        client: reqwest::Client,
        base_url: String,
        fhir_version: FhirVersion,
    ) -> Self {
        Self {
            backend: Arc::new(HeliosTerminologyBackend::with_client(
                client,
                base_url,
                fhir_version,
            )),
        }
    }

    pub fn with_backend(backend: Arc<dyn TerminologyBackend>) -> Self {
        Self { backend }
    }

    /// Full `ValueSet/$validate-code` using [`ValidateVsRequest`].
    ///
    /// Use this when you need parameters beyond what [`TerminologyService::member_of`]
    /// provides—for example `systemVersion`, embedded `coding` / `codeableConcept`,
    /// `context`, or `date`.
    pub async fn validate_vs(
        &self,
        req: &ValidateVsRequest,
    ) -> Result<TerminologyMembershipOutcome, ValidationError> {
        let response = self.backend.validate_vs(req).await?;
        parse_validate_vs_result(&response)
    }
}

#[async_trait]
impl TerminologyService for RemoteTerminologyService {
    #[instrument(skip(self, display), fields(valueset_url = %valueset_url))]
    async fn member_of(
        &self,
        valueset_url: &str,
        system: Option<&str>,
        code: &str,
        display: Option<&str>,
    ) -> Result<TerminologyMembershipOutcome, ValidationError> {
        let req = ValidateVsRequest {
            valueset_url: valueset_url.to_string(),
            code: Some(code.to_string()),
            system: system.map(str::to_string),
            display: display.map(str::to_string),
            ..Default::default()
        };

        match self.validate_vs(&req).await {
            Ok(outcome) => {
                debug!(
                    valueset_url,
                    system,
                    code,
                    ?outcome,
                    "remote terminology outcome"
                );
                Ok(outcome)
            }
            Err(e) => {
                let remote_detail: Option<&'static str> = match &e {
                    ValidationError::RemoteTerminology(r) => {
                        Some(remote_terminology_error_kind_label(r))
                    }
                    _ => None,
                };
                warn!(
                    valueset_url = %valueset_url,
                    error_kind = validation_error_kind_label(&e),
                    remote_detail = remote_detail.unwrap_or("n/a"),
                    "remote terminology member_of failed"
                );
                Err(e)
            }
        }
    }
}
