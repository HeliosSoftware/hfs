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
use helios_fhir::FhirVersion;
use parking_lot::RwLock;

use crate::error::StorageResult;
use crate::search::{IndexValue, SearchParameterExtractor, SearchParameterRegistry};
use crate::tenant::TenantContext;
use crate::types::{
    IncludeDirective, IncludeType, Page, ReverseChainedParameter, SearchBundle, SearchParamType,
    SearchParameter, SearchQuery, SearchValue, StoredResource,
};

use super::storage::ResourceStorage;

/// Reserved `id` of the synthetic `OperationOutcome` marker appended to
/// `included` when include/revinclude resolution was truncated by a
/// resource-count cap, so a Bundle stays bounded instead of unbounded (#1061).
///
/// The marker travels inside `Vec<StoredResource>` — the same channel
/// `resolve_includes`/`resolve_revincludes` already return — rather than as a
/// new `SearchResult` field, so it reaches every call site (including the
/// duplicated include-resolution block backends may carry for a sorted search
/// path) with no signature changes. Detected via [`is_include_truncation_marker`].
pub const INCLUDE_TRUNCATION_OUTCOME_ID: &str = "hfs-include-truncated";

/// Builds the synthetic truncation-marker resource described on
/// [`INCLUDE_TRUNCATION_OUTCOME_ID`].
///
/// `directive_label` should name the directive that was truncated (e.g.
/// `"_revinclude=Observation:subject"`) so the diagnostics text is
/// actionable; `limit` is the resource-count cap that was hit.
///
/// `remedy` names what the operator can actually do about it, and is supplied
/// by the caller because the cap has more than one source: a backend's own
/// configurable limit, or the fixed [`MAX_ITERATE_INCLUDED`] bound on the
/// `:iterate` continuation, which applies to every backend. Naming one
/// backend's environment variable here would misdirect callers of the other
/// path.
pub fn include_truncation_outcome(
    tenant: &TenantContext,
    fhir_version: FhirVersion,
    limit: usize,
    directive_label: &str,
    remedy: &str,
) -> StoredResource {
    let now = chrono::Utc::now();
    let content = serde_json::json!({
        "resourceType": "OperationOutcome",
        "issue": [{
            "severity": "warning",
            "code": "incomplete",
            "diagnostics": format!(
                "included resources for '{directive_label}' were truncated at {limit} \
                 resources; {remedy}"
            ),
        }]
    });
    StoredResource::from_storage(
        "OperationOutcome".to_string(),
        INCLUDE_TRUNCATION_OUTCOME_ID.to_string(),
        "1".to_string(),
        tenant.tenant_id().clone(),
        content,
        now,
        now,
        None,
        fhir_version,
    )
}

/// True when `resource` is the synthetic truncation marker built by
/// [`include_truncation_outcome`] rather than a genuine included resource.
pub fn is_include_truncation_marker(resource: &StoredResource) -> bool {
    resource.resource_type() == "OperationOutcome" && resource.id() == INCLUDE_TRUNCATION_OUTCOME_ID
}

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

    /// Converts the result to a FHIR SearchBundle, copying each resource.
    ///
    /// Prefer [`Self::into_bundle`] on a request path: the copy here is a deep
    /// clone of every matched resource's JSON, and a handler that is about to
    /// drop the `SearchResult` has no use for it.
    pub fn to_bundle(&self, base_url: &str, self_link: &str) -> SearchBundle {
        use crate::types::BundleEntry;

        let mut bundle = self.bundle_shell(self_link);

        for resource in &self.resources.items {
            let url = resource.url();
            let score = self.scores.get(&url).copied();
            bundle = bundle.with_entry(
                BundleEntry::match_entry(
                    format!("{}/{}", base_url, url),
                    resource.content_with_meta(),
                )
                .with_score(score),
            );
        }

        for resource in &self.included {
            bundle = bundle.with_entry(if is_include_truncation_marker(resource) {
                // Defensive fallback for callers that build a bundle straight
                // from `included` without draining truncation markers first
                // (the REST search handler does drain them, routing the
                // diagnostics through its own post-subsetting warning path
                // instead — see `execute_search_bundle`).
                BundleEntry::outcome_entry(resource.content().clone())
            } else {
                BundleEntry::include_entry(
                    format!("{}/{}", base_url, resource.url()),
                    resource.content_with_meta(),
                )
            });
        }

        bundle
    }

    /// Converts the result to a FHIR SearchBundle, moving each resource's JSON
    /// into its entry.
    ///
    /// This is [`Self::to_bundle`] without the copy. `StoredResource::content`
    /// is a whole `serde_json::Value` tree — for a 20-row page of Synthea
    /// Observations that clone measured 3.7 µs per resource, 73 µs per page,
    /// and it bought nothing: every request handler drops the `SearchResult`
    /// immediately afterwards.
    pub fn into_bundle(self, base_url: &str, self_link: &str) -> SearchBundle {
        use crate::types::BundleEntry;

        let mut bundle = self.bundle_shell(self_link);

        let SearchResult {
            resources,
            included,
            scores,
            ..
        } = self;

        for resource in resources.items {
            let url = resource.url();
            let score = scores.get(&url).copied();
            bundle = bundle.with_entry(
                BundleEntry::match_entry(
                    format!("{}/{}", base_url, url),
                    resource.into_content_with_meta(),
                )
                .with_score(score),
            );
        }

        for resource in included {
            if is_include_truncation_marker(&resource) {
                // See the matching comment in `to_bundle`.
                bundle = bundle.with_entry(BundleEntry::outcome_entry(resource.into_content()));
                continue;
            }
            let url = resource.url();
            bundle = bundle.with_entry(BundleEntry::include_entry(
                format!("{}/{}", base_url, url),
                resource.into_content_with_meta(),
            ));
        }

        bundle
    }

    /// Builds the bundle envelope — `total` and the `self` / `next` /
    /// `previous` / `first` links — shared by [`Self::to_bundle`] and
    /// [`Self::into_bundle`].
    fn bundle_shell(&self, self_link: &str) -> SearchBundle {
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

    /// Returns the search parameter registry **for a tenant**.
    ///
    /// Search-parameter resolution is tenant-scoped: every tenant sees the shared
    /// base params (embedded + spec + custom), plus its own stored (POSTed)
    /// params overlaid on top. So `acme1` and `acme2` may resolve searches
    /// against different parameter sets. The returned registry is the source of
    /// truth for param-type resolution (see [`crate::search::resolve_param_type`]);
    /// REST extractors and chained-search builders both consult it so they cannot
    /// disagree on whether a given param is a Date vs. Token vs. Reference, etc.
    ///
    /// Returns an owned `Arc` (cloned from the per-tenant cache); bind it to a
    /// local before `.read()`.
    fn search_param_registry(&self, tenant: &TenantContext)
    -> Arc<RwLock<SearchParameterRegistry>>;

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

/// Upper bound on the number of resources [`resolve_includes_iterate_continuation`]
/// will add, so a continuation hop over a large reverse set stays bounded the
/// same way the MongoDB backend's own cap bounds its first hop (#1061/#1063).
/// [`resolve_includes_iterative`]'s full pass keeps today's unbounded budget —
/// SQLite/Postgres behaviour is unchanged by this constant.
pub const MAX_ITERATE_INCLUDED: usize = 1000;

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
/// correctly — and the referenced resources are fetched with `search()`. Only
/// references the extractor has already resolved to a `resource_type` +
/// `resource_id` pair are followed; conditional references
/// (`Type?param=value`), `urn:` references, and contained (`#`) references
/// have no resolvable id and therefore never produce an included resource.
///
/// Runs hop 1 (every directive) then `:iterate`-only hops, exactly as before
/// #1063 — unbounded budget, so SQLite/Postgres see no behaviour change. For a
/// backend that already resolved hop 1 inline, use
/// [`resolve_includes_iterate_continuation`] instead so hop 1 is not re-issued.
pub async fn resolve_includes_iterative<S>(
    provider: &S,
    tenant: &TenantContext,
    matches: &[StoredResource],
    includes: &[IncludeDirective],
) -> StorageResult<Vec<StoredResource>>
where
    S: SearchProvider + ?Sized,
{
    resolve_includes_iterative_inner(provider, tenant, matches, includes, &[], usize::MAX).await
}

/// Continues transitive `:iterate` resolution for a backend whose `search()`
/// already resolved hop 1 inline (MongoDB, Elasticsearch — #1063).
///
/// `already_included` is what the backend's own `search()` returned in
/// `SearchResult::included` (any [`is_include_truncation_marker`] entries are
/// ignored, both as seeds and as frontier). Hop 1 is **not** re-run: the
/// frontier starts at `already_included` and only directives carrying
/// `iterate` are applied, so a resource the backend already returned is never
/// fetched or counted twice. Bounded by [`MAX_ITERATE_INCLUDED`]; hitting the
/// bound appends a synthetic truncation marker (see
/// [`include_truncation_outcome`]) rather than growing without limit.
pub async fn resolve_includes_iterate_continuation<S>(
    provider: &S,
    tenant: &TenantContext,
    matches: &[StoredResource],
    includes: &[IncludeDirective],
    already_included: &[StoredResource],
) -> StorageResult<Vec<StoredResource>>
where
    S: SearchProvider + ?Sized,
{
    resolve_includes_iterative_inner(
        provider,
        tenant,
        matches,
        includes,
        already_included,
        MAX_ITERATE_INCLUDED,
    )
    .await
}

async fn resolve_includes_iterative_inner<S>(
    provider: &S,
    tenant: &TenantContext,
    matches: &[StoredResource],
    includes: &[IncludeDirective],
    already_included: &[StoredResource],
    max_included: usize,
) -> StorageResult<Vec<StoredResource>>
where
    S: SearchProvider + ?Sized,
{
    if matches.is_empty() || includes.is_empty() {
        return Ok(Vec::new());
    }

    let tenant_registry = provider.search_param_registry(tenant);
    let extractor = SearchParameterExtractor::new(tenant_registry.clone());
    let key = |r: &StoredResource| format!("{}/{}", r.resource_type(), r.id());

    // Don't re-include primary matches, nor anything the backend already
    // returned (real resources only — never seed from a truncation marker).
    let mut seen: HashSet<String> = matches.iter().map(&key).collect();
    let real_already_included: Vec<StoredResource> = already_included
        .iter()
        .filter(|r| !is_include_truncation_marker(r))
        .cloned()
        .collect();
    for r in &real_already_included {
        seen.insert(key(r));
    }

    let mut included: Vec<StoredResource> = Vec::new();

    // No prior pass: behave exactly like the original single-entry-point
    // function (hop 1 applies every directive). A prior pass: hop 1 was
    // already done by the backend, so start the frontier there and only run
    // `:iterate` directives.
    let (mut frontier, mut first_hop): (Vec<StoredResource>, bool) =
        if real_already_included.is_empty() {
            (matches.to_vec(), true)
        } else {
            (real_already_included, false)
        };
    let mut depth = 0;
    let mut truncated_directive: Option<String> = None;

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
                        let def = tenant_registry
                            .read()
                            .get_param(res.resource_type(), &directive.search_param);
                        let Some(def) = def else { continue };
                        if let Ok(values) = extractor.extract_for_param(res.content(), &def) {
                            for v in values {
                                if let IndexValue::Reference {
                                    resource_type: Some(t),
                                    resource_id: Some(i),
                                    ..
                                } = v.value
                                {
                                    if let Some(target) = &directive.target_type {
                                        if &t != target {
                                            continue;
                                        }
                                    }
                                    wanted.push((t, i));
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
        // Bounded by `max_included` — for the full pass that bound is
        // `usize::MAX` so this never trips; for a continuation it caps the
        // total this call can add.
        let mut next = Vec::new();
        let mut hop_truncated = false;
        for r in fetched {
            let k = key(&r);
            if seen.contains(&k) {
                continue;
            }
            if included.len() >= max_included {
                hop_truncated = true;
                break;
            }
            seen.insert(k);
            next.push(r.clone());
            included.push(r);
        }
        if hop_truncated && truncated_directive.is_none() {
            let label = active_labels(includes, first_hop);
            truncated_directive = Some(label);
        }

        first_hop = false;
        depth += 1;
        frontier = next;
        if frontier.is_empty() || hop_truncated || depth >= MAX_INCLUDE_ITERATE_DEPTH {
            break;
        }
    }

    if let Some(label) = truncated_directive {
        let fhir_version = matches
            .first()
            .or(already_included.first())
            .map(|r| r.fhir_version())
            .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);
        included.push(include_truncation_outcome(
            tenant,
            fhir_version,
            max_included,
            &label,
            "the _include:iterate expansion limit is fixed and not configurable",
        ));
    }

    Ok(included)
}

/// Comma-joined `type:param[:iterate]` labels of the directives active for a
/// hop, for the truncation marker's diagnostics text.
fn active_labels(includes: &[IncludeDirective], first_hop: bool) -> String {
    includes
        .iter()
        .filter(|d| first_hop || d.iterate)
        .map(|d| {
            let kind = match d.include_type {
                IncludeType::Include => "_include",
                IncludeType::Revinclude => "_revinclude",
            };
            if d.iterate {
                format!("{kind}:iterate={}:{}", d.source_type, d.search_param)
            } else {
                format!("{kind}={}:{}", d.source_type, d.search_param)
            }
        })
        .collect::<Vec<_>>()
        .join(",")
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
    use crate::types::{PageInfo, SearchEntryMode};
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

    fn test_tenant() -> TenantContext {
        TenantContext::new(
            crate::tenant::TenantId::new("t1"),
            crate::tenant::TenantPermissions::full_access(),
        )
    }

    /// #1061: a truncation marker in `included` must become a `search.mode =
    /// outcome` entry (no `fullUrl`, no `id` on the resource) rather than an
    /// ordinary `include` entry — both via `to_bundle` and `into_bundle`. This
    /// is the defensive fallback for non-REST callers (e.g. compartment
    /// search) that build a bundle straight from `SearchResult` without
    /// draining markers themselves.
    #[test]
    fn truncation_outcome_becomes_an_outcome_entry() {
        let tenant = test_tenant();
        let patient = StoredResource::new(
            "Patient",
            "123",
            tenant.tenant_id().clone(),
            serde_json::json!({"resourceType": "Patient", "id": "123"}),
            FhirVersion::default(),
        );
        let marker = include_truncation_outcome(
            &tenant,
            FhirVersion::default(),
            5,
            "_revinclude=Observation:subject",
            "increase the limit",
        );
        assert!(is_include_truncation_marker(&marker));
        assert!(!is_include_truncation_marker(&patient));

        let page = Page::new(Vec::new(), PageInfo::end());
        let result = SearchResult::new(page).with_included(vec![patient.clone(), marker.clone()]);

        // to_bundle (borrowing)
        let bundle = result.to_bundle("http://example.com/fhir", "http://example.com/fhir/Patient");
        assert_eq!(bundle.entry.len(), 2);

        let outcome_entries: Vec<_> = bundle
            .entry
            .iter()
            .filter(|e| e.search.as_ref().map(|s| s.mode) == Some(SearchEntryMode::Outcome))
            .collect();
        assert_eq!(outcome_entries.len(), 1, "exactly one outcome entry");
        let outcome_entry = outcome_entries[0];
        assert!(
            outcome_entry.full_url.is_none(),
            "outcome entry must not carry a fullUrl"
        );
        let resource = outcome_entry.resource.as_ref().expect("resource present");
        assert_eq!(resource["resourceType"], "OperationOutcome");
        assert!(
            resource.get("id").is_none(),
            "marker resource must carry no id in the bundle (rewrite_bundle_full_urls skips id-less resources)"
        );
        assert_eq!(resource["issue"][0]["severity"], "warning");
        assert!(
            resource["issue"][0]["diagnostics"]
                .as_str()
                .unwrap()
                .contains('5'),
            "diagnostics should name the limit"
        );

        let include_entries: Vec<_> = bundle
            .entry
            .iter()
            .filter(|e| e.search.as_ref().map(|s| s.mode) == Some(SearchEntryMode::Include))
            .collect();
        assert_eq!(include_entries.len(), 1, "Patient stays an include entry");

        // into_bundle (owning) — same assertions.
        let page2 = Page::new(Vec::new(), PageInfo::end());
        let result2 = SearchResult::new(page2).with_included(vec![patient, marker]);
        let bundle2 =
            result2.into_bundle("http://example.com/fhir", "http://example.com/fhir/Patient");
        assert_eq!(bundle2.entry.len(), 2);
        let outcome_entries2: Vec<_> = bundle2
            .entry
            .iter()
            .filter(|e| e.search.as_ref().map(|s| s.mode) == Some(SearchEntryMode::Outcome))
            .collect();
        assert_eq!(outcome_entries2.len(), 1);
        assert!(outcome_entries2[0].full_url.is_none());
        assert_eq!(
            outcome_entries2[0].resource.as_ref().unwrap()["resourceType"],
            "OperationOutcome"
        );
        let include_entries2: Vec<_> = bundle2
            .entry
            .iter()
            .filter(|e| e.search.as_ref().map(|s| s.mode) == Some(SearchEntryMode::Include))
            .collect();
        assert_eq!(include_entries2.len(), 1);
    }
}
