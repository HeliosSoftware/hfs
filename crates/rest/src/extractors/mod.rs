//! Axum extractors for FHIR-specific data.
//!
//! This module provides custom Axum extractors for common FHIR patterns:
//!
//! - [`TenantExtractor`] - Extract tenant context from request
//! - [`FhirVersionExtractor`] - Extract FHIR version from headers
//! - [`FhirResource`] - Extract and validate FHIR resources
//! - [`SearchParams`] - Extract and parse search parameters
//! - [`Pagination`] - Extract pagination parameters
//! - [`search_query_builder`] - Convert REST params to persistence SearchQuery

mod fhir_resource;
mod fhir_version;
mod pagination;
pub(crate) mod query_pairs;
mod search_params;
pub mod search_query_builder;
mod tenant;

pub use fhir_resource::FhirResource;
pub use fhir_version::FhirVersionExtractor;
pub use pagination::Pagination;
pub use search_params::SearchParams;
pub use search_query_builder::{
    build_search_query, build_search_query_from_map, build_search_query_from_pairs,
};
pub use tenant::TenantExtractor;
