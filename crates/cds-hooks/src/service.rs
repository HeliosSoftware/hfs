//! Traits for implementing CDS Hooks services.
//!
//! This module defines the core [`CdsHooksService`] trait that developers implement
//! to create CDS Hooks decision support services. The trait follows an async pattern
//! compatible with Axum, Actix, and other Rust web frameworks.
//!
//! # Architecture
//!
//! A CDS Hooks server typically hosts one or more services, each responding to a
//! specific hook (e.g. `patient-view`, `order-sign`). The server exposes:
//!
//! - **Discovery endpoint** (`GET /cds-services`) — lists all available services
//! - **Service endpoint** (`POST /cds-services/{id}`) — invokes a specific service
//! - **Feedback endpoint** (`POST /cds-services/{id}/feedback`) — receives outcome data
//!
//! Implement [`CdsHooksService`] for each service, then register them with your
//! HTTP framework of choice.
//!
//! # Example
//!
//! ```rust
//! use async_trait::async_trait;
//! use helios_cds_hooks::{
//!     CdsHooksService, CdsRequest, CdsResponse, CdsService, Card,
//!     FeedbackRequest, CdsHooksError,
//!     hooks::{HookContext, PatientViewContext},
//! };
//! use std::collections::HashMap;
//!
//! struct PatientGreeter;
//!
//! #[async_trait]
//! impl CdsHooksService for PatientGreeter {
//!     type Context = PatientViewContext;
//!
//!     fn definition(&self) -> CdsService {
//!         CdsService {
//!             hook: PatientViewContext::HOOK_NAME.to_string(),
//!             title: Some("Patient Greeter".to_string()),
//!             description: "Greets the patient when their chart is opened".to_string(),
//!             id: "patient-greeter".to_string(),
//!             prefetch: Some(HashMap::from([(
//!                 "patientToGreet".to_string(),
//!                 "Patient/{{context.patientId}}".to_string(),
//!             )])),
//!             usage_requirements: None,
//!             version: None,
//!             extension: None,
//!         }
//!     }
//!
//!     async fn call(
//!         &self,
//!         _request: &CdsRequest,
//!         context: &PatientViewContext,
//!     ) -> Result<CdsResponse, CdsHooksError> {
//!         let card = Card::info(
//!             format!("Hello, patient {}!", context.patient_id),
//!             "Patient Greeter",
//!         );
//!         Ok(CdsResponse::with_cards(vec![card]))
//!     }
//! }
//! ```

use async_trait::async_trait;

use crate::hooks::HookContext;
use crate::models::{CdsRequest, CdsResponse, CdsService, FeedbackRequest};

/// Errors that can occur when processing a CDS Hooks request.
#[derive(Debug, thiserror::Error)]
pub enum CdsHooksError {
    /// The CDS Service cannot retrieve the necessary FHIR data to execute its
    /// decision support (HTTP 412 Precondition Failed).
    #[error("precondition failed: {0}")]
    PreconditionFailed(String),

    /// The request context could not be deserialized into the expected hook context type.
    #[error("invalid context: {0}")]
    InvalidContext(#[from] serde_json::Error),

    /// An internal error occurred while processing the request.
    #[error("internal error: {0}")]
    InternalError(String),
}

impl CdsHooksError {
    /// Returns the appropriate HTTP status code for this error.
    pub fn status_code(&self) -> u16 {
        match self {
            CdsHooksError::PreconditionFailed(_) => 412,
            CdsHooksError::InvalidContext(_) => 400,
            CdsHooksError::InternalError(_) => 500,
        }
    }
}

/// Trait for implementing a CDS Hooks service.
///
/// Each implementation represents a single CDS service that responds to a specific
/// hook type. The service declares its metadata through [`definition`](CdsHooksService::definition),
/// processes hook requests through [`call`](CdsHooksService::call), and optionally
/// handles feedback through [`on_feedback`](CdsHooksService::on_feedback).
///
/// # Type Parameters
///
/// - `Context`: The hook context type this service responds to (e.g. [`PatientViewContext`],
///   [`OrderSignContext`]). This determines which hook triggers the service and what
///   contextual data is available.
///
/// # Extracting Context
///
/// The [`call`](CdsHooksService::call) method receives both the raw [`CdsRequest`] (for
/// access to prefetch data, FHIR authorization, etc.) and the deserialized hook context.
/// Use [`extract_context`](CdsHooksService::extract_context) to parse the context from
/// a raw request if needed.
///
/// [`PatientViewContext`]: crate::hooks::PatientViewContext
/// [`OrderSignContext`]: crate::hooks::OrderSignContext
#[async_trait]
pub trait CdsHooksService: Send + Sync {
    /// The hook context type this service processes.
    type Context: HookContext;

    /// Returns the service definition for the discovery endpoint.
    ///
    /// This metadata is included in the response to `GET {baseUrl}/cds-services`.
    fn definition(&self) -> CdsService;

    /// Processes a CDS Hooks request and returns decision support cards.
    ///
    /// # Arguments
    ///
    /// - `request`: The full CDS Hooks request, including prefetch data and
    ///   FHIR authorization information.
    /// - `context`: The deserialized hook context.
    ///
    /// # Errors
    ///
    /// Return [`CdsHooksError::PreconditionFailed`] if required prefetch data is
    /// missing and the service cannot fetch it directly. Return other error variants
    /// for invalid input or internal failures.
    async fn call(
        &self,
        request: &CdsRequest,
        context: &Self::Context,
    ) -> Result<CdsResponse, CdsHooksError>;

    /// Handles feedback about what happened with previously returned cards.
    ///
    /// The default implementation does nothing. Override this to track suggestion
    /// acceptance rates, override reasons, and other analytics.
    ///
    /// # Arguments
    ///
    /// - `feedback`: The feedback request containing outcome data for one or more cards.
    async fn on_feedback(&self, _feedback: &FeedbackRequest) -> Result<(), CdsHooksError> {
        Ok(())
    }

    /// Extracts and deserializes the hook context from a raw CDS request.
    ///
    /// This is a convenience method that parses the `context` field of the request
    /// into the service's expected context type.
    fn extract_context(&self, request: &CdsRequest) -> Result<Self::Context, CdsHooksError> {
        Ok(serde_json::from_value(request.context.clone())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::PatientViewContext;
    use crate::models::Card;
    use std::collections::HashMap;

    struct TestService;

    #[async_trait]
    impl CdsHooksService for TestService {
        type Context = PatientViewContext;

        fn definition(&self) -> CdsService {
            CdsService {
                hook: PatientViewContext::HOOK_NAME.to_string(),
                title: Some("Test Service".to_string()),
                description: "A test service".to_string(),
                id: "test-service".to_string(),
                prefetch: Some(HashMap::from([(
                    "patient".to_string(),
                    "Patient/{{context.patientId}}".to_string(),
                )])),
                usage_requirements: None,
                version: None,
                extension: None,
            }
        }

        async fn call(
            &self,
            _request: &CdsRequest,
            context: &PatientViewContext,
        ) -> Result<CdsResponse, CdsHooksError> {
            let card = Card::info(
                format!("Hello, patient {}!", context.patient_id),
                "Test Service",
            );
            Ok(CdsResponse::with_cards(vec![card]))
        }
    }

    #[tokio::test]
    async fn test_service_definition() {
        let service = TestService;
        let def = service.definition();
        assert_eq!(def.hook, "patient-view");
        assert_eq!(def.id, "test-service");
    }

    #[tokio::test]
    async fn test_service_call() {
        let service = TestService;
        let request = CdsRequest {
            hook: "patient-view".to_string(),
            hook_instance: "test-instance".to_string(),
            fhir_server: None,
            fhir_authorization: None,
            context: serde_json::json!({
                "userId": "Practitioner/123",
                "patientId": "456"
            }),
            prefetch: None,
            extension: None,
        };

        let context = service.extract_context(&request).unwrap();
        assert_eq!(context.patient_id, "456");

        let response = service.call(&request, &context).await.unwrap();
        assert_eq!(response.cards.len(), 1);
        assert!(response.cards[0].summary.contains("456"));
    }

    #[tokio::test]
    async fn test_extract_context_error() {
        let service = TestService;
        let request = CdsRequest {
            hook: "patient-view".to_string(),
            hook_instance: "test-instance".to_string(),
            fhir_server: None,
            fhir_authorization: None,
            context: serde_json::json!({"wrongField": "value"}),
            prefetch: None,
            extension: None,
        };

        let result = service.extract_context(&request);
        assert!(result.is_err());
    }

    #[test]
    fn test_error_status_codes() {
        assert_eq!(
            CdsHooksError::PreconditionFailed("missing data".to_string()).status_code(),
            412
        );
        assert_eq!(
            CdsHooksError::InternalError("oops".to_string()).status_code(),
            500
        );
    }

    #[tokio::test]
    async fn test_default_feedback_handler() {
        let service = TestService;
        let feedback = FeedbackRequest { feedback: vec![] };
        let result = service.on_feedback(&feedback).await;
        assert!(result.is_ok());
    }
}
