//! SearchProvider, TextSearchProvider, IncludeProvider, and RevincludeProvider
//! implementations for the Elasticsearch backend.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use elasticsearch::SearchParts;
use serde_json::{Value, json};
use tokio::time::sleep;

use crate::core::ResourceStorage;
use crate::core::search::{
    IncludeProvider, RevincludeProvider, SearchProvider, SearchResult, TextSearchProvider,
};
use crate::error::{BackendError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    CursorDirection, CursorValue, IncludeDirective, Page, PageCursor, PageInfo, Pagination,
    SearchQuery, StoredResource,
};

use super::backend::ElasticsearchBackend;
use super::schema;
use super::search::fts;
use super::search::query_builder::{EsQueryBuilder, build_count_query};

fn internal_error(message: String) -> crate::error::StorageError {
    crate::error::StorageError::Backend(BackendError::Internal {
        backend_name: "elasticsearch".to_string(),
        message,
        source: None,
    })
}

/// The cluster could not be reached (connection refused, DNS, TLS, timeout).
///
/// This is deliberately distinct from [`internal_error`]: "I could not ask" is
/// not "the answer is no". Callers must never turn this into an empty result —
/// see the `SearchAttempt::Unreachable` docs.
fn unavailable_error(message: String) -> crate::error::StorageError {
    crate::error::StorageError::Backend(BackendError::Unavailable {
        backend_name: "elasticsearch".to_string(),
        message,
    })
}

/// Maximum retry attempts for transient ES search failures (in addition to the
/// initial attempt). Transient failures observed in CI: shard allocation
/// flapping during recovery/relocation, brief master-node hiccups.
const MAX_SEARCH_RETRIES: u32 = 2;

/// Initial backoff before retrying a transient ES error. Doubled per attempt.
const RETRY_BASE_DELAY_MS: u64 = 100;

/// Returns true if an ES failure response indicates a transient,
/// safe-to-retry condition rather than a permanent error.
///
/// `no_shard_available_action_exception` and `search_phase_execution_exception`
/// are documented as retryable; HTTP 503 covers the general "service
/// unavailable" case (often surfaced when shards are still recovering).
fn is_transient_es_error(status: u16, body: &str) -> bool {
    status == 503
        || body.contains("no_shard_available_action_exception")
        || body.contains("search_phase_execution_exception")
}

/// Result of a single search attempt: either a parsed body, an empty
/// "index does not exist" sentinel, or a (possibly transient) failure.
enum SearchAttempt {
    Body(Value),
    EmptyIndex,
    /// The cluster was unreachable at the transport layer (connection refused,
    /// DNS failure, TLS failure, timeout) — we never got an answer at all.
    ///
    /// This is retried like a transient error (a rolling restart or a
    /// not-yet-ready cluster is a real, recoverable condition), but once the
    /// retries are exhausted it MUST surface as an error. It must never
    /// degrade into an empty result set: a search that reports "no matches"
    /// when it never reached the cluster is indistinguishable from a search
    /// that genuinely found nothing, and for a FHIR query ("does this patient
    /// have any allergies?") those two answers are not interchangeable.
    Unreachable(String),
    Transient {
        status: u16,
        body: String,
    },
    Permanent(crate::error::StorageError),
}

/// Sends a single ES search request and classifies the response.
async fn send_search_once(
    backend: &ElasticsearchBackend,
    index: &str,
    body: Value,
) -> SearchAttempt {
    let response = backend
        .client()
        .search(SearchParts::Index(&[index]))
        .body(body)
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            // Transport-level failure: we never reached the cluster, so we do
            // not know whether the index exists or what it contains. Retry it
            // (see `SearchAttempt::Unreachable`), but never report it as an
            // empty index — that would silently turn "Elasticsearch is down"
            // into "this patient has no matching records".
            tracing::warn!("ES search request failed at the transport layer: {}", e);
            return SearchAttempt::Unreachable(e.to_string());
        }
    };

    if response.status_code().is_success() {
        return match response.json::<Value>().await {
            Ok(v) => SearchAttempt::Body(v),
            Err(e) => SearchAttempt::Permanent(internal_error(format!(
                "Failed to parse search response: {}",
                e
            ))),
        };
    }

    let status = response.status_code().as_u16();
    let resp_body = response.text().await.unwrap_or_default();

    if resp_body.contains("index_not_found_exception") {
        return SearchAttempt::EmptyIndex;
    }

    if is_transient_es_error(status, &resp_body) {
        SearchAttempt::Transient {
            status,
            body: resp_body,
        }
    } else {
        SearchAttempt::Permanent(internal_error(format!("Search failed: {}", resp_body)))
    }
}

/// A retryable failure carried across attempts so the last one can be reported.
enum RetryableFailure {
    /// Cluster never answered (transport failure).
    Unreachable(String),
    /// Cluster answered with a retryable status/body.
    Transient { status: u16, body: String },
}

/// Sends an ES search and retries on transient errors with exponential backoff.
///
/// Returns:
/// - `Ok(Some(value))` — successful response, parsed JSON body
/// - `Ok(None)` — index does not exist (caller returns empty results)
/// - `Err(...)` — non-transient failure, or retries exhausted
///
/// `Ok(None)` is returned ONLY for a genuine `index_not_found_exception`. An
/// unreachable cluster always yields `Err`, never `Ok(None)` — the caller turns
/// `Ok(None)` into an empty result set, and an empty result set is a factual
/// claim about the data that we are in no position to make when we never
/// reached the cluster.
async fn send_search_with_retry(
    backend: &ElasticsearchBackend,
    index: &str,
    body: Value,
) -> StorageResult<Option<Value>> {
    let mut last_failure: Option<RetryableFailure> = None;

    for attempt in 0..=MAX_SEARCH_RETRIES {
        let failure = match send_search_once(backend, index, body.clone()).await {
            SearchAttempt::Body(v) => return Ok(Some(v)),
            SearchAttempt::EmptyIndex => return Ok(None),
            SearchAttempt::Permanent(e) => return Err(e),
            SearchAttempt::Unreachable(message) => RetryableFailure::Unreachable(message),
            SearchAttempt::Transient { status, body } => {
                RetryableFailure::Transient { status, body }
            }
        };

        if attempt < MAX_SEARCH_RETRIES {
            let delay_ms = RETRY_BASE_DELAY_MS << attempt;
            tracing::warn!(
                attempt = attempt + 1,
                max = MAX_SEARCH_RETRIES + 1,
                delay_ms,
                index,
                "Retryable ES search failure, retrying"
            );
            sleep(Duration::from_millis(delay_ms)).await;
        }
        last_failure = Some(failure);
    }

    let attempts = MAX_SEARCH_RETRIES + 1;
    Err(
        match last_failure.expect("a retryable branch always sets last_failure") {
            RetryableFailure::Unreachable(message) => unavailable_error(format!(
                "Elasticsearch unreachable after {attempts} attempts: {message}"
            )),
            RetryableFailure::Transient { status, body } => internal_error(format!(
                "Search failed after {attempts} attempts (status {status}): {body}"
            )),
        },
    )
}

/// Converts an Elasticsearch hit's `sort` array into cursor values, dropping
/// the trailing tie-breaker (the resource id the query builder appends to
/// every sort for deterministic ordering). The remaining values are mapped
/// to the matching [`CursorValue`] variant so the cursor can be replayed
/// against a future search without re-deriving them from the stored
/// resource.
fn cursor_values_from_sort(sort_values: &[Value]) -> Vec<CursorValue> {
    sort_values
        .iter()
        .take(sort_values.len().saturating_sub(1))
        .map(|v| {
            if let Some(s) = v.as_str() {
                CursorValue::String(s.to_string())
            } else if let Some(n) = v.as_i64() {
                CursorValue::Number(n)
            } else if let Some(b) = v.as_bool() {
                CursorValue::Boolean(b)
            } else if v.is_null() {
                CursorValue::Null
            } else {
                CursorValue::String(v.to_string())
            }
        })
        .collect()
}

/// Builds an opaque page cursor pointing at `resource`, or `None` if the hit
/// carried no `sort` values (e.g. the query had no explicit or default sort
/// applied to it). `direction` selects whether the cursor should be replayed
/// as `Next` (walk forward from this resource) or `Previous` (walk backward
/// from this resource).
fn page_cursor_for(
    resource: &StoredResource,
    sort_values: Option<&Vec<Value>>,
    direction: CursorDirection,
) -> Option<String> {
    let values = cursor_values_from_sort(sort_values?);
    let cursor = match direction {
        CursorDirection::Next => PageCursor::new(values, resource.id()),
        CursorDirection::Previous => PageCursor::previous(values, resource.id()),
    };
    Some(cursor.encode())
}

#[async_trait]
impl SearchProvider for ElasticsearchBackend {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        // `_contained` search post-processes contained-doc hits into containers or
        // contained resources; standard search excludes contained docs via the
        // query builder's `must_not is_contained`.
        if query.contained != crate::types::ContainedMode::Off {
            return self.search_contained(tenant, query).await;
        }

        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = &query.resource_type;
        let index = self.index_name(tenant_id, resource_type);

        // Build ES query
        let builder = EsQueryBuilder::new(tenant_id, resource_type, index.clone());
        let es_query = builder.build(query);

        // Execute search (with retry on transient shard-availability errors)
        let body = match send_search_with_retry(self, &index, es_query.body).await? {
            Some(v) => v,
            None => return Ok(SearchResult::new(Page::new(vec![], PageInfo::end()))),
        };

        // Parse hits
        let hits = body
            .get("hits")
            .and_then(|h| h.get("hits"))
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default();

        let total = body
            .get("hits")
            .and_then(|h| h.get("total"))
            .and_then(|t| t.get("value"))
            .and_then(|v| v.as_u64());

        let count = query.count.unwrap_or(20) as usize;

        let mut hits_with_sort: Vec<(StoredResource, Option<Vec<Value>>)> = Vec::new();
        let mut scores: HashMap<String, f64> = HashMap::new();

        for hit in &hits {
            let source = match hit.get("_source") {
                Some(s) => s,
                None => continue,
            };

            // Skip deleted
            if source
                .get("is_deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            if let Some(stored) = parse_hit_to_stored_resource(source, tenant)? {
                // Capture the relevance score for `Bundle.entry.search.score`.
                // `_score` is null when a field sort overrides relevance scoring,
                // so only record finite scores.
                if let Some(score) = hit.get("_score").and_then(|s| s.as_f64()) {
                    scores.insert(stored.url(), score);
                }
                let sort_values = hit.get("sort").and_then(Value::as_array).cloned();
                hits_with_sort.push((stored, sort_values));
            }
        }

        // A `Previous` cursor asks the query builder for the reversed sort order
        // (T1) plus one extra hit (`count + 1`), so `hits_with_sort` here arrives
        // in reverse result order and may be one item longer than the requested
        // page. That extra hit is the farthest one from the cursor — it belongs
        // to the page *before* the one we return, not to this page — so it must
        // be dropped with `truncate` before we `reverse()` back into normal
        // query order. See #1015.
        let backward = query
            .cursor
            .as_deref()
            .and_then(|c| PageCursor::decode(c).ok())
            .is_some_and(|c| c.direction() == CursorDirection::Previous);

        let page_info = if backward {
            let has_previous = hits_with_sort.len() > count;
            if has_previous {
                hits_with_sort.truncate(count);
            }
            hits_with_sort.reverse();

            if hits_with_sort.is_empty() {
                PageInfo {
                    next_cursor: None,
                    previous_cursor: None,
                    total,
                    has_next: false,
                    has_previous: false,
                }
            } else {
                let next_cursor = hits_with_sort
                    .last()
                    .and_then(|(r, s)| page_cursor_for(r, s.as_ref(), CursorDirection::Next));
                let previous_cursor = if has_previous {
                    hits_with_sort.first().and_then(|(r, s)| {
                        page_cursor_for(r, s.as_ref(), CursorDirection::Previous)
                    })
                } else {
                    None
                };
                PageInfo {
                    next_cursor,
                    previous_cursor,
                    total,
                    has_next: true,
                    has_previous,
                }
            }
        } else {
            let has_next = hits_with_sort.len() >= count;
            let has_previous = query.cursor.is_some() || query.offset.unwrap_or(0) > 0;
            let next_cursor = if has_next {
                hits_with_sort
                    .last()
                    .and_then(|(r, s)| page_cursor_for(r, s.as_ref(), CursorDirection::Next))
            } else {
                None
            };
            let previous_cursor = if has_previous {
                hits_with_sort
                    .first()
                    .and_then(|(r, s)| page_cursor_for(r, s.as_ref(), CursorDirection::Previous))
            } else {
                None
            };
            PageInfo {
                next_cursor,
                previous_cursor,
                total,
                has_next,
                has_previous,
            }
        };

        let resources: Vec<StoredResource> = hits_with_sort.into_iter().map(|(r, _)| r).collect();
        let page = Page::new(resources, page_info);
        let mut result = SearchResult::new(page);

        if !scores.is_empty() {
            result = result.with_scores(scores);
        }

        if let Some(t) = total {
            result = result.with_total(t);
        }

        Ok(result)
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = &query.resource_type;
        let index = self.index_name(tenant_id, resource_type);

        let count_body = build_count_query(tenant_id, resource_type, query);

        let response = self
            .client()
            .count(elasticsearch::CountParts::Index(&[&index]))
            .body(count_body)
            .send()
            .await;

        // A count of 0 is a factual claim about the data. Only make it when the
        // cluster actually told us so (success), or when the index genuinely
        // does not exist yet (404). A transport failure or a 5xx means we never
        // got an answer, and must surface as an error rather than as "zero".
        match response {
            Ok(resp) if resp.status_code().is_success() => {
                let body: Value = resp.json().await.unwrap_or_default();
                Ok(body.get("count").and_then(|c| c.as_u64()).unwrap_or(0))
            }
            Ok(resp) if resp.status_code().as_u16() == 404 => Ok(0),
            Ok(resp) => {
                let status = resp.status_code().as_u16();
                let body = resp.text().await.unwrap_or_default();
                Err(internal_error(format!(
                    "Count failed (status {status}): {body}"
                )))
            }
            Err(e) => Err(unavailable_error(format!(
                "Elasticsearch unreachable during count: {e}"
            ))),
        }
    }

    fn search_param_registry(
        &self,
        tenant: &crate::tenant::TenantContext,
    ) -> std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        self.tenant_registries()
            .for_tenant(tenant.tenant_id().as_str())
    }

    fn supports_contained_search(&self) -> bool {
        true
    }

    fn modifiers_for_param_type(
        &self,
        param_type: crate::types::SearchParamType,
    ) -> Vec<&'static str> {
        Self::modifiers_for_type(param_type)
    }
}

impl ElasticsearchBackend {
    /// Executes a `_contained=true|both` search. The query builder restricts the
    /// hit set (`is_contained=true` for `on`; no restriction for `both`); this
    /// post-processes each hit: contained-doc hits resolve to their container
    /// (`_containedType=container`, default) or the contained resource itself
    /// (`_containedType=contained`), while top-level hits (only present for
    /// `both`) pass through. Single window (no keyset cursor).
    async fn search_contained(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        use crate::types::ContainedReturn;

        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = &query.resource_type;
        let index = self.index_name(tenant_id, resource_type);

        // Fetch a generous window of candidate hits (offset/count applied below).
        let mut es_query =
            EsQueryBuilder::new(tenant_id, resource_type, index.clone()).build(query);
        let count = query.count.unwrap_or(100) as usize;
        let offset = query.offset.unwrap_or(0) as usize;
        if let Some(obj) = es_query.body.as_object_mut() {
            obj.insert("size".to_string(), json!(offset + count));
            obj.remove("from");
            obj.remove("search_after");
        }

        let body = match send_search_with_retry(self, &index, es_query.body).await? {
            Some(v) => v,
            None => return Ok(SearchResult::new(Page::new(vec![], PageInfo::end()))),
        };
        let hits = body
            .get("hits")
            .and_then(|h| h.get("hits"))
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default();

        let mut items: Vec<StoredResource> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for hit in &hits {
            let Some(source) = hit.get("_source") else {
                continue;
            };
            if source
                .get("is_deleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }

            let is_contained = source
                .get("is_contained")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if !is_contained {
                // Top-level hit (only in `both` mode) — pass through.
                if let Some(stored) = parse_hit_to_stored_resource(source, tenant)? {
                    if seen.insert(stored.url()) {
                        items.push(stored);
                    }
                }
                continue;
            }

            let (Some(container_type), Some(container_id)) = (
                source.get("container_type").and_then(|v| v.as_str()),
                source.get("container_id").and_then(|v| v.as_str()),
            ) else {
                continue;
            };

            match query.contained_return {
                ContainedReturn::Container => {
                    if !seen.insert(format!("{container_type}/{container_id}")) {
                        continue;
                    }
                    if let Some(container) = self.read(tenant, container_type, container_id).await?
                    {
                        items.push(container);
                    }
                }
                ContainedReturn::Contained => {
                    // The contained doc's `content` IS the contained resource;
                    // return it directly with its local id.
                    if let Some(stored) = parse_hit_to_stored_resource(source, tenant)? {
                        let local_id = source
                            .get("contained_local_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_else(|| stored.id());
                        let key = format!("{container_type}/{container_id}#{local_id}");
                        if seen.insert(key) {
                            let rebuilt = StoredResource::from_storage(
                                stored.resource_type().to_string(),
                                local_id.to_string(),
                                stored.version_id().to_string(),
                                tenant.tenant_id().clone(),
                                stored.content().clone(),
                                stored.created_at(),
                                stored.last_modified(),
                                None,
                                stored.fhir_version(),
                            );
                            items.push(rebuilt);
                        }
                    }
                }
            }
        }

        // Apply the offset/count window.
        let total = if query.wants_total() {
            Some(items.len() as u64)
        } else {
            None
        };
        let windowed: Vec<StoredResource> = items.into_iter().skip(offset).take(count).collect();
        let page = Page::new(windowed, PageInfo::end());
        let mut result = SearchResult::new(page);
        if let Some(t) = total {
            result = result.with_total(t);
        }
        Ok(result)
    }
}

#[async_trait]
impl TextSearchProvider for ElasticsearchBackend {
    async fn search_text(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        text: &str,
        pagination: &Pagination,
    ) -> StorageResult<SearchResult> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);

        schema::ensure_index(self, tenant_id, resource_type).await?;

        let body = json!({
            "query": {
                "bool": {
                    "must": [fts::build_narrative_query(text)],
                    "filter": [
                        { "term": { "tenant_id": tenant_id } },
                        { "term": { "is_deleted": false } }
                    ]
                }
            },
            "size": pagination.count,
            "track_total_hits": true,
            "sort": [
                { "_score": { "order": "desc" } },
                { "resource_id": { "order": "asc" } }
            ]
        });

        execute_text_search(self, &index, body, tenant).await
    }

    async fn search_content(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        content: &str,
        pagination: &Pagination,
    ) -> StorageResult<SearchResult> {
        let tenant_id = tenant.tenant_id().as_str();
        let index = self.index_name(tenant_id, resource_type);

        schema::ensure_index(self, tenant_id, resource_type).await?;

        let body = json!({
            "query": {
                "bool": {
                    "must": [fts::build_content_query(content)],
                    "filter": [
                        { "term": { "tenant_id": tenant_id } },
                        { "term": { "is_deleted": false } }
                    ]
                }
            },
            "size": pagination.count,
            "track_total_hits": true,
            "sort": [
                { "_score": { "order": "desc" } },
                { "resource_id": { "order": "asc" } }
            ]
        });

        execute_text_search(self, &index, body, tenant).await
    }
}

/// Executes a text search query and returns the results.
async fn execute_text_search(
    backend: &ElasticsearchBackend,
    index: &str,
    body: Value,
    tenant: &TenantContext,
) -> StorageResult<SearchResult> {
    let body = match send_search_with_retry(backend, index, body).await? {
        Some(v) => v,
        None => return Ok(SearchResult::new(Page::new(vec![], PageInfo::end()))),
    };

    let hits = body
        .get("hits")
        .and_then(|h| h.get("hits"))
        .and_then(|h| h.as_array())
        .cloned()
        .unwrap_or_default();

    let total = body
        .get("hits")
        .and_then(|h| h.get("total"))
        .and_then(|t| t.get("value"))
        .and_then(|v| v.as_u64());

    let mut resources = Vec::new();
    for hit in &hits {
        if let Some(source) = hit.get("_source") {
            if let Some(stored) = parse_hit_to_stored_resource(source, tenant)? {
                resources.push(stored);
            }
        }
    }

    let page = Page::new(resources, PageInfo::end());
    let mut result = SearchResult::new(page);
    if let Some(t) = total {
        result = result.with_total(t);
    }
    Ok(result)
}

#[async_trait]
impl IncludeProvider for ElasticsearchBackend {
    /// Delegates to the shared, registry-driven resolver so `_include` (and
    /// `:iterate`) follows the same search-parameter definitions (with FHIRPath
    /// expression evaluation) used to build the index, instead of a
    /// backend-specific reference extractor that could disagree with it.
    async fn resolve_includes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        crate::core::resolve_includes_iterative(self, tenant, resources, includes).await
    }
}

#[async_trait]
impl RevincludeProvider for ElasticsearchBackend {
    async fn resolve_revincludes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        revincludes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        let mut result = Vec::new();

        for directive in revincludes {
            let source_type = &directive.source_type;
            if source_type.is_empty() {
                continue;
            }

            for resource in resources {
                let reference_value = format!("{}/{}", resource.resource_type(), resource.id());

                // Search for resources of source_type that reference this resource
                let query =
                    SearchQuery::new(source_type).with_parameter(crate::types::SearchParameter {
                        name: directive.search_param.clone(),
                        param_type: crate::types::SearchParamType::Reference,
                        modifier: None,
                        values: vec![crate::types::SearchValue::eq(&reference_value)],
                        chain: vec![],
                        components: vec![],
                    });

                let search_result = self.search(tenant, &query).await?;

                for stored in search_result.resources.items {
                    if !result.iter().any(|r: &StoredResource| {
                        r.resource_type() == stored.resource_type() && r.id() == stored.id()
                    }) {
                        result.push(stored);
                    }
                }
            }
        }

        Ok(result)
    }
}

/// Parses an ES hit's `_source` into a `StoredResource`.
fn parse_hit_to_stored_resource(
    source: &Value,
    tenant: &TenantContext,
) -> StorageResult<Option<StoredResource>> {
    let resource_type = match source.get("resource_type").and_then(|v| v.as_str()) {
        Some(rt) => rt,
        None => return Ok(None),
    };

    let resource_id = match source.get("resource_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return Ok(None),
    };

    let version_id = source
        .get("version_id")
        .and_then(|v| v.as_str())
        .unwrap_or("1");

    let content = source.get("content").cloned().unwrap_or_else(|| json!({}));

    let fhir_version_str = source
        .get("fhir_version")
        .and_then(|v| v.as_str())
        .unwrap_or("4.0");
    let fhir_version = helios_fhir::FhirVersion::from_mime_param(fhir_version_str)
        .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

    let last_updated = source
        .get("last_updated")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    Ok(Some(StoredResource::from_storage(
        resource_type,
        resource_id,
        version_id,
        tenant.tenant_id().clone(),
        content,
        last_updated,
        last_updated,
        None,
        fhir_version,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_es_error_classification() {
        // Real failure body observed in CI (HFS log):
        let no_shard = r#"{"error":{"root_cause":[{"type":"no_shard_available_action_exception","reason":"..."}],"type":"search_phase_execution_exception","reason":"all shards failed"},"status":503}"#;
        assert!(is_transient_es_error(500, no_shard));
        assert!(is_transient_es_error(503, ""));
        assert!(is_transient_es_error(
            500,
            r#"{"error":{"type":"search_phase_execution_exception"}}"#
        ));

        // Permanent failures must not be retried.
        assert!(!is_transient_es_error(
            400,
            r#"{"error":{"type":"parsing_exception"}}"#
        ));
        assert!(!is_transient_es_error(
            500,
            r#"{"error":{"type":"illegal_argument_exception"}}"#
        ));
        assert!(!is_transient_es_error(404, "index_not_found_exception"));
    }

    #[test]
    fn cursor_values_from_sort_drops_tie_breaker_and_maps_types() {
        let sort_values = vec![
            json!(1700),
            json!("x"),
            json!(true),
            Value::Null,
            json!("p-1"),
        ];

        let cursor_values = cursor_values_from_sort(&sort_values);

        assert_eq!(cursor_values.len(), 4);
        assert!(matches!(cursor_values[0], CursorValue::Number(1700)));
        assert!(matches!(&cursor_values[1], CursorValue::String(s) if s == "x"));
        assert!(matches!(cursor_values[2], CursorValue::Boolean(true)));
        assert!(matches!(cursor_values[3], CursorValue::Null));
    }
}
