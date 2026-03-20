//! # helios-cds-hooks
//!
//! Rust types and traits for building [CDS Hooks](https://cds-hooks.hl7.org/) services
//! conforming to the HL7 CDS Hooks specification (v3.0.0-ballot).
//!
//! CDS Hooks is a standard for invoking clinical decision support from within a
//! clinician's workflow. A CDS Client (typically an EHR) calls CDS Services at
//! specific points called *hooks*, and receives *cards* containing information,
//! suggested actions, and links.
//!
//! ## Features
//!
//! - **Complete protocol types** — All request, response, discovery, card, suggestion,
//!   action, link, and feedback types with full serde support
//! - **Strongly-typed hook contexts** — Dedicated structs for every hook in the
//!   [CDS Hooks Library](https://cds-hooks.hl7.org/hooks/), with compile-time validation
//! - **Service trait** — Async trait for implementing CDS services, compatible with
//!   any Rust web framework (Axum, Actix, etc.)
//! - **Builder helpers** — Convenience methods for constructing cards and responses
//!
//! ## Quick Start
//!
//! ```rust
//! use async_trait::async_trait;
//! use helios_cds_hooks::{
//!     CdsHooksService, CdsRequest, CdsResponse, CdsService, Card, CdsHooksError,
//!     hooks::{HookContext, PatientViewContext},
//! };
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
//!             description: "Greets the patient".to_string(),
//!             id: "patient-greeter".to_string(),
//!             prefetch: None,
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
//!         Ok(CdsResponse::with_cards(vec![
//!             Card::info(
//!                 format!("Hello, patient {}!", context.patient_id),
//!                 "Patient Greeter",
//!             ),
//!         ]))
//!     }
//! }
//! ```
//!
//! ## Supported Hooks
//!
//! All hooks from the CDS Hooks Library are supported:
//!
//! | Hook | Context Type | Maturity |
//! |------|-------------|----------|
//! | `patient-view` | [`PatientViewContext`](hooks::PatientViewContext) | 5 - Mature |
//! | `order-select` | [`OrderSelectContext`](hooks::OrderSelectContext) | 4 - Documented |
//! | `order-sign` | [`OrderSignContext`](hooks::OrderSignContext) | 5 - Mature |
//! | `encounter-start` | [`EncounterStartContext`](hooks::EncounterStartContext) | 1 - Submitted |
//! | `encounter-discharge` | [`EncounterDischargeContext`](hooks::EncounterDischargeContext) | 1 - Submitted |
//! | `appointment-book` | [`AppointmentBookContext`](hooks::AppointmentBookContext) | 1 - Submitted |
//! | `order-dispatch` | [`OrderDispatchContext`](hooks::OrderDispatchContext) | 0 - Draft |
//! | `allergyintolerance-create` | [`AllergyIntoleranceCreateContext`](hooks::AllergyIntoleranceCreateContext) | 1 - Submitted |
//! | `medication-refill` | [`MedicationRefillContext`](hooks::MedicationRefillContext) | 1 - Submitted |
//! | `problem-list-item-create` | [`ProblemListItemCreateContext`](hooks::ProblemListItemCreateContext) | 1 - Submitted |
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────┐         ┌──────────────┐         ┌──────────────────┐
//! │  CDS Client │─hook──▶ │  CDS Server  │─calls──▶│ CdsHooksService  │
//! │   (EHR)     │◀─cards─ │  (your app)  │◀─resp── │ (your impl)      │
//! └─────────────┘         └──────────────┘         └──────────────────┘
//!       │                        │
//!       │                        ▼
//!       │                 GET /cds-services ──▶ DiscoveryResponse
//!       │                 POST /cds-services/{id} ──▶ CdsResponse
//!       └─feedback──────▶ POST /cds-services/{id}/feedback
//! ```

pub mod hooks;
pub mod models;
pub mod service;

// Re-export primary types at the crate root for convenience.
pub use hooks::{
    AllergyIntoleranceCreateContext, AppointmentBookContext, EncounterDischargeContext,
    EncounterStartContext, MedicationRefillContext, OrderDispatchContext, OrderSelectContext,
    OrderSignContext, PatientViewContext, ProblemListItemCreateContext,
};
pub use models::{
    AcceptedSuggestion, Action, ActionSelectionBehavior, ActionType, Card, CdsRequest, CdsResponse,
    CdsService, Coding, DiscoveryResponse, Feedback, FeedbackOutcome, FeedbackRequest,
    FhirAuthorization, Indicator, Link, LinkType, OverrideReason, SelectionBehavior, Source,
    Suggestion,
};
pub use service::{CdsHooksError, CdsHooksService};
