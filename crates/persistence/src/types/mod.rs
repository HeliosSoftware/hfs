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
    ChainConfig, ChainedParameter, CompartmentMembership, CompositeSearchComponent, ContainedMode,
    ContainedReturn, IncludeDirective, IncludeType, ReverseChainedParameter, SearchModifier,
    SearchParamType, SearchParameter, SearchPrefix, SearchQuery, SearchValue, SortDirection,
    SortDirective, SummaryMode, TotalMode, strip_reference_version,
};

pub use stored_resource::{ResourceMeta, ResourceMethod, StoredResource, StoredResourceBuilder};

pub use search_capabilities::{
    ChainingCapability, CompositeComponent, DatePrecision, IncludeCapability, IndexingMode,
    JsonbCapabilities, PaginationCapability, ResultModeCapability, SearchParamFullCapability,
    SearchStrategy, SpecialSearchParam,
};

/// A server-assigned FHIR resource id.
///
/// Time-ordered (UUID version 7) rather than random (version 4), because every
/// index in this schema that is keyed by a resource's id is a b-tree, and a
/// random key inserts into it at a random leaf.
///
/// `search_index` is the case that matters: `idx_search_resource` is
/// `(tenant_id, resource_type, resource_id)`, every one of a resource's ~14
/// index rows enters it, and the benchmark's import suite writes 22.3 million
/// of them. With random ids each of those insertions lands on an arbitrary page
/// of a 500 MB index against a 2 GB buffer pool — a cache miss, a dirtied page,
/// and its share of page splits. With time-ordered ids they arrive at the
/// right-hand edge of each `(tenant, resource_type)` slice, which is already
/// resident.
///
/// Measured on PostgreSQL 18.6 with `shared_buffers = 192MB`, 4.2M rows of the
/// same shape (300,000 resources x 14 rows), `COPY` into an identical table
/// carrying that index, two rounds each:
///
/// ```text
/// random  (v4)   13.93 s   13.97 s
/// ordered (v7)    7.86 s    9.92 s
/// ```
///
/// **-36% on the whole copy**, and the index half of it is roughly -60% —
/// about 13% of the `INSERT INTO search_index` statement, which is 89% of the
/// import suite's Postgres time and half of the crud suite's. `resources`'s own
/// primary key `(tenant_id, resource_type, id)` gets the same treatment.
///
/// The index ends up slightly larger (90 MB against 71 MB here): an ascending
/// key splits a leaf and leaves the left half half-full, where a random key
/// comes back and fills it. That is a few percent of footprint against a third
/// of the write cost.
///
/// # What this changes for a client
///
/// A version 7 UUID embeds the millisecond it was minted, so the *creation
/// time* of a server-assigned id is now recoverable from the id. That is
/// already public in `meta.lastUpdated` on the resource itself, and the
/// remaining 74 bits are random, so an id is no more guessable than before.
/// Client-supplied ids are untouched — this is only the fallback for a `POST`
/// that does not carry one — and ids already stored keep whatever form they
/// have.
pub fn new_resource_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

#[cfg(test)]
mod resource_id_tests {
    use super::new_resource_id;

    /// Ids stay unique, stay valid FHIR ids, and stay ordered by mint time —
    /// the ordering is the whole point (see [`new_resource_id`]).
    #[test]
    fn ids_are_unique_valid_and_ordered() {
        let ids: Vec<String> = (0..2_000).map(|_| new_resource_id()).collect();

        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "ids must be unique");

        for id in &ids {
            // FHIR `id`: 1..64 of [A-Za-z0-9\-\.]
            assert!((1..=64).contains(&id.len()), "{id} has a bad length");
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'),
                "{id} is not a valid FHIR id"
            );
            let parsed = uuid::Uuid::parse_str(id).expect("parses as a UUID");
            assert_eq!(parsed.get_version_num(), 7, "{id} is not a v7 UUID");
        }

        // The 48-bit millisecond prefix is monotonic, so the string form sorts
        // by mint time. Equal is allowed: many ids share a millisecond.
        let mut sorted = ids.clone();
        sorted.sort();
        let prefix = |s: &String| s[..8].to_string();
        assert_eq!(
            ids.iter().map(prefix).collect::<Vec<_>>(),
            sorted.iter().map(prefix).collect::<Vec<_>>(),
            "the timestamp prefix must already be in ascending order"
        );
    }
}
