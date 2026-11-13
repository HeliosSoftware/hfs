//! Response formatting for the FHIR REST API.
//!
//! This module provides utilities for building FHIR-compliant responses:
//!
//! - [`operation_outcome`] - OperationOutcome generation
//! - [`bundle`] - Bundle response building
//! - [`headers`] - Response header generation (ETag, Location, etc.)
//! - [`subsetting`] - Resource subsetting for _summary and _elements

pub mod bundle;
pub mod format;
pub mod headers;
pub mod operation_outcome;
pub mod subsetting;

pub use bundle::BundleBuilder;
pub use format::{format_resource_response, format_resource_response_for_version};
pub use headers::ResourceHeaders;
pub use operation_outcome::OperationOutcomeBuilder;
pub use subsetting::{SummaryMode, apply_elements, apply_summary};
