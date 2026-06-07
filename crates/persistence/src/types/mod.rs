//! Core types for the persistence layer.
//!
//! This module provides the fundamental types used throughout the persistence layer:
//!
//! - [`StoredResource`] - A FHIR resource with persistence metadata
//! - [`SearchParameter`], [`SearchQuery`] - Search parameter types
//! - [`Pagination`], [`PageCursor`] - Pagination types
//! - [`SearchBundle`] - FHIR Bundle for search results
//!
//! # Examples
//!
//! ## Creating a Stored Resource
//!
//! ```
//! use helios_persistence::types::StoredResource;
//! use helios_persistence::tenant::TenantId;
//! use helios_fhir::FhirVersion;
//! use serde_json::json;
//!
//! let resource = StoredResource::new(
//!     "Patient",
//!     "patient-123",
//!     TenantId::new("acme"),
//!     json!({
//!         "resourceType": "Patient",
//!         "id": "patient-123",
//!         "name": [{"family": "Smith", "given": ["John"]}]
//!     }),
//!     FhirVersion::default(),
//! );
//!
//! assert_eq!(resource.url(), "Patient/patient-123");
//! assert_eq!(resource.version_id(), "1");
//! ```
//!
//! ## Building a Search Query
//!
//! ```
//! use helios_persistence::types::{
//!     SearchQuery, SearchParameter, SearchParamType, SearchValue, SortDirective
//! };
//!
//! let query = SearchQuery::new("Patient")
//!     .with_parameter(SearchParameter {
//!         name: "name".to_string(),
//!         param_type: SearchParamType::String,
//!         modifier: None,
//!         values: vec![SearchValue::eq("Smith")],
//!         chain: vec![],
//!         components: vec![],
//!     })
//!     .with_sort(SortDirective::parse("-_lastUpdated"))
//!     .with_count(20);
//! ```
//!
//! ## Pagination
//!
//! ```
//! use helios_persistence::types::{Pagination, PageCursor, CursorValue};
//!
//! // Cursor-based pagination (recommended)
//! let pagination = Pagination::cursor().with_count(50);
//!
//! // Create a cursor for the next page
//! let cursor = PageCursor::new(
//!     vec![CursorValue::from("2024-01-15T10:30:00Z")],
//!     "resource-id",
//! );
//! let encoded = cursor.encode();
//!
//! // Parse cursor from request
//! let decoded = PageCursor::decode(&encoded).unwrap();
//! ```

mod pagination;
mod search_capabilities;
mod search_params;
mod stored_resource;

pub use pagination::{
    BundleEntry, BundleEntrySearch, BundleLink, CursorDirection, CursorValue, Page, PageCursor,
    PageInfo, Pagination, PaginationMode, SearchBundle, SearchEntryMode,
};

pub use search_params::{
    ChainConfig, ChainedParameter, CompositeSearchComponent, IncludeDirective, IncludeType,
    ReverseChainedParameter, SearchModifier, SearchParamType, SearchParameter, SearchPrefix,
    SearchQuery, SearchValue, SortDirection, SortDirective, SummaryMode, TotalMode,
    strip_reference_version,
};

pub use stored_resource::{ResourceMeta, ResourceMethod, StoredResource, StoredResourceBuilder};

pub use search_capabilities::{
    ChainingCapability, CompositeComponent, DatePrecision, IncludeCapability, IndexingMode,
    JsonbCapabilities, PaginationCapability, ResultModeCapability, SearchParamFullCapability,
    SearchStrategy, SpecialSearchParam,
};
