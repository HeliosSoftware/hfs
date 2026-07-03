//! Search provider traits.
//!
//! This module defines a hierarchy of search provider traits:
//! - [`SearchProvider`] - Basic single-type search
//! - [`MultiTypeSearchProvider`] - Search across multiple resource types
//! - [`IncludeProvider`] - Support for _include
//! - [`RevincludeProvider`] - Support for _revinclude
//! - [`ChainedSearchProvider`] - Chained parameters and _has
//! - [`TerminologySearchProvider`] - :above, :below, :in, :not-in
//! - [`TextSearchProvider`] - Full-text search (_text, _content, :text)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use crate::error::StorageResult;
use crate::search::{IndexValue, SearchParameterExtractor, SearchParameterRegistry};
use crate::tenant::TenantContext;
use crate::types::{
    IncludeDirective, IncludeType, Page, ReverseChainedParameter, SearchBundle, SearchParamType,
    SearchParameter, SearchQuery, SearchValue, StoredResource,
};

use super::storage::ResourceStorage;

/// Result of a search operation.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// The matching resources.
    pub resources: Page<StoredResource>,

    /// Included resources (from _include/_revinclude).
    pub included: Vec<StoredResource>,

    /// Total count of matches (if requested via _total).
    pub total: Option<u64>,

    /// Relevance scores (`Bundle.entry.search.score`) for matched resources,
    /// keyed by resource URL (`Type/id`). Populated by backends that compute
    /// relevance (e.g. Elasticsearch full-text search); empty otherwise.
    pub scores: HashMap<String, f64>,
}

impl SearchResult {
    /// Creates a new search result.
    pub fn new(resources: Page<StoredResource>) -> Self {
        Self {
            resources,
            included: Vec::new(),
            total: None,
            scores: HashMap::new(),
        }
    }

    /// Adds included resources.
    pub fn with_included(mut self, included: Vec<StoredResource>) -> Self {
        self.included = included;
        self
    }

    /// Sets the total count.
    pub fn with_total(mut self, total: u64) -> Self {
        self.total = Some(total);
        self
    }

    /// Sets the relevance scores, keyed by resource URL (`Type/id`).
    pub fn with_scores(mut self, scores: HashMap<String, f64>) -> Self {
        self.scores = scores;
        self
    }

    /// Returns the number of matching resources in this page.
    pub fn len(&self) -> usize {
        self.resources.len()
    }

    /// Returns true if there are no matching resources.
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }

    /// Returns the cursor for the next page, if there is one.
    pub fn next_cursor(&self) -> Option<&String> {
        self.resources.page_info.next_cursor.as_ref()
    }

    /// Returns the cursor for the previous page, if there is one.
    pub fn previous_cursor(&self) -> Option<&String> {
        self.resources.page_info.previous_cursor.as_ref()
    }

    /// Returns whether there are more results after this page.
    pub fn has_next(&self) -> bool {
        self.resources.page_info.has_next
    }

    /// Returns whether there are results before this page.
    pub fn has_previous(&self) -> bool {
        self.resources.page_info.has_previous
    }

    /// Converts the result to a FHIR SearchBundle.
    pub fn to_bundle(&self, base_url: &str, self_link: &str) -> SearchBundle {
        use crate::types::{BundleEntry, SearchBundle};

        let mut bundle = SearchBundle::new().with_self_link(self_link);

        if let Some(total) = self.total {
            bundle = bundle.with_total(total);
        }

        // Add next link if there's more data. The self_link already contains
        // the request's query string (potentially including a `_cursor` param
        // from the previous page), so we strip any existing `_cursor=` and
        // append the new one with the correct delimiter.
        if let Some(ref cursor) = self.resources.page_info.next_cursor {
            bundle = bundle.with_next_link(replace_cursor_param(self_link, cursor));
        }

        if let Some(ref cursor) = self.resources.page_info.previous_cursor {
            bundle = bundle.with_previous_link(replace_cursor_param(self_link, cursor));
        }

        // First-page link: the self URL with paging params (`_cursor` / `_offset`)
        // stripped. Emitted only for multi-page results (when a next/previous page
        // exists). A `last` link is intentionally not emitted: under keyset
        // (cursor) paging the final page is not cheaply computable.
        if self.resources.page_info.next_cursor.is_some()
            || self.resources.page_info.previous_cursor.is_some()
        {
            bundle = bundle.with_link("first", strip_paging_params(self_link));
        }

        // Add matching resources, attaching a relevance score when the backend
        // computed one for this resource (`Bundle.entry.search.score`).
        for resource in &self.resources.items {
            let full_url = format!("{}/{}", base_url, resource.url());
            let entry = BundleEntry::match_entry(full_url, resource.content().clone())
                .with_score(self.scores.get(&resource.url()).copied());
            bundle = bundle.with_entry(entry);
        }

        // Add included resources
        for resource in &self.included {
            let full_url = format!("{}/{}", base_url, resource.url());
            bundle = bundle.with_entry(BundleEntry::include_entry(
                full_url,
                resource.content().clone(),
            ));
        }

        bundle
    }
}

/// Returns `url` with any existing `_cursor=…` query parameter replaced by the
/// supplied opaque `cursor` value. Used to build pagination links from the
/// request's self URL.
///
/// Cursors are base64-url-safe so they don't need percent-encoding; the only
/// surgery required is splitting on the first `?`, dropping any pre-existing
/// `_cursor` pair, and re-joining with `&`. This is what makes the difference
/// between
///
/// ```text
/// .../Patient?_count=3&_elements=id?_cursor=…   // wrong: literal `?` mid-query
/// ```
///
/// and the spec-compliant
///
/// ```text
/// .../Patient?_count=3&_elements=id&_cursor=…
/// ```
/// Returns `url` with any `_cursor=…` and `_offset=…` query parameters removed,
/// yielding the first-page URL for a paginated search.
fn strip_paging_params(url: &str) -> String {
    let (base, query) = match url.find('?') {
        Some(pos) => (&url[..pos], &url[pos + 1..]),
        None => return url.to_string(),
    };

    let parts: Vec<String> = query
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with("_cursor=") && !p.starts_with("_offset="))
        .map(str::to_string)
        .collect();

    if parts.is_empty() {
        base.to_string()
    } else {
        format!("{}?{}", base, parts.join("&"))
    }
}

fn replace_cursor_param(url: &str, cursor: &str) -> String {
    let (base, query) = match url.find('?') {
        Some(pos) => (&url[..pos], &url[pos + 1..]),
        None => (url, ""),
    };

    let mut parts: Vec<String> = query
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with("_cursor="))
        .map(str::to_string)
        .collect();
    parts.push(format!("_cursor={}", cursor));

    format!("{}?{}", base, parts.join("&"))
}

/// Basic search provider for single resource type queries.
///
/// This trait provides search functionality for a single resource type,
/// corresponding to the FHIR search interaction:
/// `GET [base]/[type]?[parameters]`
///
/// # Example
///
/// ```ignore
/// use helios_persistence::core::SearchProvider;
/// use helios_persistence::types::{SearchQuery, SearchParameter, SearchParamType, SearchValue};
///
/// async fn search_patients<S: SearchProvider>(
///     storage: &S,
///     tenant: &TenantContext,
/// ) -> Result<(), StorageError> {
///     let query = SearchQuery::new("Patient")
///         .with_parameter(SearchParameter {
///             name: "name".to_string(),
///             param_type: SearchParamType::String,
///             modifier: None,
///             values: vec![SearchValue::eq("Smith")],
///             chain: vec![],
///             components: vec![],
///         })
///         .with_count(20);
///
///     let result = storage.search(tenant, &query).await?;
///
///     for resource in result.resources.items {
///         println!("Found: {}", resource.url());
///     }
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait SearchProvider: ResourceStorage {
    /// Searches for resources matching the query.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `query` - The search query with parameters
    ///
    /// # Returns
    ///
    /// A search result with matching resources and pagination info.
    ///
    /// # Errors
    ///
    /// * `StorageError::Validation` - If the query contains invalid parameters
    /// * `StorageError::Search` - If a search feature is not supported
    /// * `StorageError::Tenant` - If the tenant doesn't have search permission
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult>;

    /// Counts resources matching the query without returning them.
    ///
    /// This is more efficient than search when you only need the count.
    async fn search_count(&self, tenant: &TenantContext, query: &SearchQuery)
    -> StorageResult<u64>;

    /// Returns the backend's search parameter registry.
    ///
    /// The registry is the single source of truth for search parameter type
    /// resolution (see [`crate::search::resolve_param_type`]). REST extractors
    /// and chained-search builders both consult it so they cannot disagree on
    /// whether a given param is a Date vs. Token vs. Reference, etc.
    fn search_param_registry(&self) -> &Arc<RwLock<SearchParameterRegistry>>;

    /// Whether this backend can evaluate `_contained=true|both` searches (which
    /// require contained-resource indexing). Defaults to `false`; backends that
    /// index `contained[]` entries override this. The REST layer uses it to
    /// reject `_contained` with `501` on backends that don't support it rather
    /// than silently returning an unfiltered result.
    fn supports_contained_search(&self) -> bool {
        false
    }

    /// Returns the search modifiers this backend actually honors for a given
    /// parameter type (e.g. `exact`, `contains` for strings; `not`, `in` for
    /// tokens). Used by the REST layer to advertise supported modifiers in the
    /// CapabilityStatement.
    ///
    /// Defaults to an empty list (advertise nothing); real search backends
    /// override this to reflect what their search implementation accepts so the
    /// CapabilityStatement stays honest.
    fn modifiers_for_param_type(&self, param_type: SearchParamType) -> Vec<&'static str> {
        let _ = param_type;
        Vec::new()
    }
}

/// Search provider that supports searching across multiple resource types.
///
/// This extends [`SearchProvider`] to support system-level search:
/// `GET [base]?[parameters]`
#[async_trait]
pub trait MultiTypeSearchProvider: SearchProvider {
    /// Searches across multiple resource types.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_types` - The resource types to search (empty = all types)
    /// * `query` - The search query
    ///
    /// # Returns
    ///
    /// A search result with matching resources from all specified types.
    async fn search_multi(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
        query: &SearchQuery,
    ) -> StorageResult<SearchResult>;
}

/// Search provider that supports _include.
///
/// _include adds referenced resources to the search results.
#[async_trait]
pub trait IncludeProvider: SearchProvider {
    /// Resolves _include directives for search results.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resources` - The primary search results
    /// * `includes` - The include directives to resolve
    ///
    /// # Returns
    ///
    /// Resources referenced by the primary results according to the include directives.
    async fn resolve_includes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>>;
}

/// Search provider that supports _revinclude.
///
/// _revinclude adds resources that reference the search results.
#[async_trait]
pub trait RevincludeProvider: SearchProvider {
    /// Resolves _revinclude directives for search results.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resources` - The primary search results
    /// * `revincludes` - The revinclude directives to resolve
    ///
    /// # Returns
    ///
    /// Resources that reference the primary results according to the revinclude directives.
    async fn resolve_revincludes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        revincludes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>>;
}

/// Maximum number of `:iterate` hops when transitively following includes,
/// guarding against reference cycles.
const MAX_INCLUDE_ITERATE_DEPTH: usize = 5;

/// Upper bound on resources fetched per internal include/revinclude query, to
/// avoid the default page size silently truncating included resources.
const INCLUDE_FETCH_LIMIT: u32 = 10_000;

/// Resolves `_include`/`_revinclude` directives for a set of primary matches,
/// following `:iterate` directives transitively until no new resources are
/// found (bounded by [`MAX_INCLUDE_ITERATE_DEPTH`]). Included resources are
/// deduplicated by `type/id` and never include a primary match.
///
/// This is the single, backend-agnostic include-resolution path used by the
/// REST layer for backends whose `search()` does not resolve includes inline
/// (SQLite, Postgres). References are extracted via the search-parameter
/// registry's FHIRPath expression — so parameters whose name differs from the
/// JSON field (e.g. Patient `organization` → `managingOrganization`) resolve
/// correctly — and the referenced resources are fetched with `search()`.
pub async fn resolve_includes_iterative<S>(
    provider: &S,
    tenant: &TenantContext,
    matches: &[StoredResource],
    includes: &[IncludeDirective],
) -> StorageResult<Vec<StoredResource>>
where
    S: SearchProvider + ?Sized,
{
    if matches.is_empty() || includes.is_empty() {
        return Ok(Vec::new());
    }

    let extractor = SearchParameterExtractor::new(provider.search_param_registry().clone());
    let key = |r: &StoredResource| format!("{}/{}", r.resource_type(), r.id());

    // Don't re-include primary matches.
    let mut seen: HashSet<String> = matches.iter().map(&key).collect();
    let mut included: Vec<StoredResource> = Vec::new();

    let mut frontier: Vec<StoredResource> = matches.to_vec();
    let mut first_hop = true;
    let mut depth = 0;

    loop {
        // First hop applies all directives; later hops only `:iterate` ones.
        let active: Vec<&IncludeDirective> =
            includes.iter().filter(|d| first_hop || d.iterate).collect();
        if active.is_empty() {
            break;
        }

        let mut fetched: Vec<StoredResource> = Vec::new();
        for directive in active {
            match directive.include_type {
                IncludeType::Include => {
                    // Forward: collect references from the frontier resources,
                    // then fetch the referenced resources by id.
                    let mut wanted: Vec<(String, String)> = Vec::new();
                    for res in &frontier {
                        if res.resource_type() != directive.source_type {
                            continue;
                        }
                        let def = provider
                            .search_param_registry()
                            .read()
                            .get_param(res.resource_type(), &directive.search_param);
                        let Some(def) = def else { continue };
                        if let Ok(values) = extractor.extract_for_param(res.content(), &def) {
                            for v in values {
                                if let IndexValue::Reference { reference, .. } = v.value {
                                    if let Some((t, i)) = reference.split_once('/') {
                                        if let Some(target) = &directive.target_type {
                                            if t != target {
                                                continue;
                                            }
                                        }
                                        wanted.push((t.to_string(), i.to_string()));
                                    }
                                }
                            }
                        }
                    }
                    // Group ids by type and fetch each group.
                    let mut by_type: std::collections::HashMap<String, Vec<String>> =
                        std::collections::HashMap::new();
                    for (t, i) in wanted {
                        by_type.entry(t).or_default().push(i);
                    }
                    for (rtype, ids) in by_type {
                        let mut q = SearchQuery::new(&rtype).with_parameter(SearchParameter {
                            name: "_id".to_string(),
                            param_type: SearchParamType::Token,
                            modifier: None,
                            values: ids.iter().map(SearchValue::eq).collect(),
                            chain: vec![],
                            components: vec![],
                        });
                        q.count = Some(INCLUDE_FETCH_LIMIT);
                        let result = provider.search(tenant, &q).await?;
                        fetched.extend(result.resources.items);
                    }
                }
                IncludeType::Revinclude => {
                    // Reverse: find source resources that reference any frontier
                    // resource via the directive's reference parameter.
                    let refs: Vec<SearchValue> =
                        frontier.iter().map(|r| SearchValue::eq(key(r))).collect();
                    if refs.is_empty() {
                        continue;
                    }
                    let mut q =
                        SearchQuery::new(&directive.source_type).with_parameter(SearchParameter {
                            name: directive.search_param.clone(),
                            param_type: SearchParamType::Reference,
                            modifier: None,
                            values: refs,
                            chain: vec![],
                            components: vec![],
                        });
                    q.count = Some(INCLUDE_FETCH_LIMIT);
                    let result = provider.search(tenant, &q).await?;
                    fetched.extend(result.resources.items);
                }
            }
        }

        // Dedup against everything seen so far; newly-added become next frontier.
        let mut next = Vec::new();
        for r in fetched {
            if seen.insert(key(&r)) {
                next.push(r.clone());
                included.push(r);
            }
        }

        first_hop = false;
        depth += 1;
        frontier = next;
        if frontier.is_empty() || depth >= MAX_INCLUDE_ITERATE_DEPTH {
            break;
        }
    }

    Ok(included)
}

/// Search provider that supports chained parameters and _has.
///
/// Chained parameters search on referenced resources:
/// `Observation?patient.name=Smith`
///
/// _has searches for resources referenced by other resources:
/// `Patient?_has:Observation:patient:code=1234-5`
#[async_trait]
pub trait ChainedSearchProvider: SearchProvider {
    /// Evaluates a chained search and returns matching resource IDs.
    ///
    /// This is used internally to resolve chains before the main search.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `base_type` - The base resource type being searched
    /// * `chain` - The chain path (e.g., "patient.organization.name")
    /// * `value` - The value to match
    ///
    /// # Returns
    ///
    /// IDs of base resources that match the chain condition.
    async fn resolve_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        chain: &str,
        value: &str,
    ) -> StorageResult<Vec<String>>;

    /// Evaluates a reverse chain (_has) and returns matching resource IDs.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `base_type` - The base resource type being searched
    /// * `reverse_chain` - The reverse chain parameters
    ///
    /// # Returns
    ///
    /// IDs of base resources that are referenced by matching resources.
    async fn resolve_reverse_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        reverse_chain: &ReverseChainedParameter,
    ) -> StorageResult<Vec<String>>;
}

/// Search provider that supports terminology-aware modifiers.
///
/// These modifiers require integration with a terminology service:
/// - `:above` - Match codes above in the hierarchy
/// - `:below` - Match codes below in the hierarchy
/// - `:in` - Match codes in a value set
/// - `:not-in` - Match codes not in a value set
#[async_trait]
pub trait TerminologySearchProvider: SearchProvider {
    /// Expands a value set and returns member codes.
    ///
    /// # Arguments
    ///
    /// * `value_set_url` - The canonical URL of the value set
    ///
    /// # Returns
    ///
    /// A list of (system, code) pairs in the value set.
    async fn expand_value_set(&self, value_set_url: &str) -> StorageResult<Vec<(String, String)>>;

    /// Gets codes above the given code in the hierarchy.
    ///
    /// # Arguments
    ///
    /// * `system` - The code system URL
    /// * `code` - The code to find ancestors for
    ///
    /// # Returns
    ///
    /// Codes that are ancestors of the given code (including the code itself).
    async fn codes_above(&self, system: &str, code: &str) -> StorageResult<Vec<String>>;

    /// Gets codes below the given code in the hierarchy.
    ///
    /// # Arguments
    ///
    /// * `system` - The code system URL
    /// * `code` - The code to find descendants for
    ///
    /// # Returns
    ///
    /// Codes that are descendants of the given code (including the code itself).
    async fn codes_below(&self, system: &str, code: &str) -> StorageResult<Vec<String>>;
}

/// Search provider that supports full-text search.
///
/// Full-text search operations:
/// - `_text` - Search in the narrative
/// - `_content` - Search in the entire resource content
/// - `:text` modifier - Full-text search on token parameters
#[async_trait]
pub trait TextSearchProvider: SearchProvider {
    /// Performs a full-text search on resource narratives.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The resource type to search
    /// * `text` - The text to search for
    /// * `pagination` - Pagination settings
    ///
    /// # Returns
    ///
    /// Resources with matching narrative text, ordered by relevance.
    async fn search_text(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        text: &str,
        pagination: &crate::types::Pagination,
    ) -> StorageResult<SearchResult>;

    /// Performs a full-text search on entire resource content.
    ///
    /// # Arguments
    ///
    /// * `tenant` - The tenant context for this operation
    /// * `resource_type` - The resource type to search
    /// * `content` - The content to search for
    /// * `pagination` - Pagination settings
    ///
    /// # Returns
    ///
    /// Resources with matching content, ordered by relevance.
    async fn search_content(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        content: &str,
        pagination: &crate::types::Pagination,
    ) -> StorageResult<SearchResult>;
}

/// Marker trait for search providers that support all advanced features.
///
/// This is a convenience trait that combines all search capabilities.
pub trait FullSearchProvider:
    SearchProvider
    + MultiTypeSearchProvider
    + IncludeProvider
    + RevincludeProvider
    + ChainedSearchProvider
{
}

// Blanket implementation for types that implement all required traits
impl<T> FullSearchProvider for T where
    T: SearchProvider
        + MultiTypeSearchProvider
        + IncludeProvider
        + RevincludeProvider
        + ChainedSearchProvider
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PageInfo;
    use helios_fhir::FhirVersion;

    #[test]
    fn test_search_result_creation() {
        let page = Page::new(Vec::new(), PageInfo::end());
        let result = SearchResult::new(page);
        assert!(result.included.is_empty());
        assert!(result.total.is_none());
    }

    #[test]
    fn test_search_result_with_included() {
        let page = Page::new(Vec::new(), PageInfo::end());
        let result = SearchResult::new(page)
            .with_included(vec![StoredResource::new(
                "Patient",
                "123",
                crate::tenant::TenantId::new("t1"),
                serde_json::json!({}),
                FhirVersion::default(),
            )])
            .with_total(100);

        assert_eq!(result.included.len(), 1);
        assert_eq!(result.total, Some(100));
    }

    #[test]
    fn test_search_result_to_bundle() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            crate::tenant::TenantId::new("t1"),
            serde_json::json!({"resourceType": "Patient", "id": "123"}),
            FhirVersion::default(),
        );

        let page = Page::new(vec![resource], PageInfo::end());
        let result = SearchResult::new(page).with_total(1);

        let bundle = result.to_bundle("http://example.com/fhir", "http://example.com/fhir/Patient");

        assert_eq!(bundle.total, Some(1));
        assert_eq!(bundle.entry.len(), 1);
        // No score recorded -> entry.search.score stays absent.
        assert!(bundle.entry[0].search.as_ref().unwrap().score.is_none());
    }

    #[test]
    fn test_search_result_to_bundle_attaches_score() {
        let resource = StoredResource::new(
            "Patient",
            "123",
            crate::tenant::TenantId::new("t1"),
            serde_json::json!({"resourceType": "Patient", "id": "123"}),
            FhirVersion::default(),
        );
        let page = Page::new(vec![resource], PageInfo::end());
        let mut scores = HashMap::new();
        scores.insert("Patient/123".to_string(), 0.875);
        let result = SearchResult::new(page).with_scores(scores);

        let bundle = result.to_bundle("http://example.com/fhir", "http://example.com/fhir/Patient");

        assert_eq!(bundle.entry.len(), 1);
        assert_eq!(
            bundle.entry[0].search.as_ref().unwrap().score,
            Some(0.875),
            "the matched entry carries Bundle.entry.search.score"
        );
    }

    /// Self-link has no query: cursor is appended with `?` (issue #69 bug 1).
    #[test]
    fn test_replace_cursor_param_no_query() {
        let url = replace_cursor_param("http://example.com/fhir/Patient", "abc");
        assert_eq!(url, "http://example.com/fhir/Patient?_cursor=abc");
    }

    /// Self-link already has params: cursor is joined with `&`, not `?`. This is
    /// the core regression — see issue #69. A literal `?` mid-query made
    /// `urljoin` percent-encode the cursor delimiter, breaking pagination.
    #[test]
    fn test_replace_cursor_param_with_existing_params() {
        let url = replace_cursor_param(
            "http://example.com/fhir/Patient?_count=3&_elements=id",
            "abc",
        );
        assert_eq!(
            url,
            "http://example.com/fhir/Patient?_count=3&_elements=id&_cursor=abc"
        );
    }

    /// When the self-link already carries the previous page's cursor (because
    /// the request URL is reused as the self link), the old cursor is dropped
    /// before the new one is appended — otherwise pages accumulate stale
    /// cursors and the next link grows unbounded.
    #[test]
    fn test_replace_cursor_param_replaces_existing_cursor() {
        let url = replace_cursor_param(
            "http://example.com/fhir/Patient?_count=3&_cursor=old&_elements=id",
            "new",
        );
        assert!(url.starts_with("http://example.com/fhir/Patient?"));
        assert!(url.contains("_count=3"));
        assert!(url.contains("_elements=id"));
        assert!(url.contains("_cursor=new"));
        assert!(!url.contains("_cursor=old"));
        assert_eq!(url.matches("_cursor=").count(), 1);
    }

    /// `to_bundle` should produce a `next` link whose URL contains exactly one
    /// `_cursor` and uses `&` between query params.
    #[test]
    fn test_to_bundle_next_link_format() {
        let page = Page::new(
            Vec::<StoredResource>::new(),
            PageInfo {
                next_cursor: Some("CURSOR_VALUE".to_string()),
                previous_cursor: None,
                total: None,
                has_next: true,
                has_previous: false,
            },
        );
        let result = SearchResult::new(page);

        let bundle = result.to_bundle(
            "http://example.com/fhir",
            "http://example.com/fhir/Patient?_count=3&_elements=id",
        );

        let next = bundle
            .link
            .iter()
            .find(|l| l.relation == "next")
            .expect("next link present");
        assert_eq!(
            next.url,
            "http://example.com/fhir/Patient?_count=3&_elements=id&_cursor=CURSOR_VALUE"
        );
        assert_eq!(
            next.url.matches('?').count(),
            1,
            "exactly one '?' delimiter"
        );
    }
}
