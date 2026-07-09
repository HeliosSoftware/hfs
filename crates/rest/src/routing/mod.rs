//! Route configuration for the FHIR REST API.
//!
//! This module contains the routing configuration that maps HTTP paths
//! to handlers.

pub mod console_metrics;
pub mod fhir_routes;

pub use fhir_routes::create_routes;
