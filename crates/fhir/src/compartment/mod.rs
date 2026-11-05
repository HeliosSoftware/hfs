//! FHIR CompartmentDefinition spec data.
//!
//! Companion to [`crate::search`]: where SearchParameters drive search
//! resolution, CompartmentDefinitions describe the compartment membership the
//! REST compartment-search handler enforces (via the codegen'd
//! [`crate::get_compartment_params`] table). This module only *loads* the spec
//! resources so the server can seed them into storage — the source of truth the
//! `GET /CompartmentDefinition` route and the web UI read from. Membership
//! itself stays codegen-driven.
//!
//! - [`loader`] – [`CompartmentDefinitionLoader`] for reading the spec bundle.

pub mod loader;

pub use loader::CompartmentDefinitionLoader;
