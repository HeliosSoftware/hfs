//! Search and conditional-operation implementation for MongoDB backend.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Utc};
use helios_fhir::FhirVersion;
use mongodb::{
    Cursor,
    bson::{self, Bson, DateTime as BsonDateTime, Document, doc},
};
use regex::escape as regex_escape;
use serde_json::Value;

use crate::core::{
    ConditionalCreateResult, ConditionalDeleteResult, ConditionalPatchResult, ConditionalStorage,
    ConditionalUpdateResult, IncludeProvider, PatchFormat, ResourceStorage, RevincludeProvider,
    SearchProvider, SearchResult,
};
use crate::error::{BackendError, QueryErrorExt, SearchError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    CompartmentMembership, CursorDirection, CursorValue, IncludeDirective, IncludeType, Page,
    PageCursor, PageInfo, SearchModifier, SearchParamType, SearchParameter, SearchPrefix,
    SearchQuery, SearchValue, StoredResource, strip_reference_version,
};

use super::MongoBackend;

/// Candidate ids per `$in` chunk when the parameter-sort aggregation is
/// bounded to a matched set (#1040). Keeps each aggregate command well under
/// the 16 MB BSON limit (10 000 UUID-length ids ≈ 0.5 MB).
const SORT_ID_CHUNK: usize = 10_000;

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "mongodb".to_string(),
        message,
        source: None,
    })
}

fn serialization_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::SerializationError { message })
}

fn bson_to_chrono(dt: &BsonDateTime) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(dt.timestamp_millis()).unwrap_or_else(Utc::now)
}

fn chrono_to_bson(dt: DateTime<Utc>) -> BsonDateTime {
    BsonDateTime::from_millis(dt.timestamp_millis())
}

/// The exclusive end of the period a partial date names: `1995` → 1996-01-01,
/// `1995-10` → 1995-11-01, `1995-10-02` → 1995-10-03. `None` for values that
/// carry a time component — those are instants, not periods.
fn implied_period_end(raw: &str, start: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match raw.len() {
        4 => Some(
            start
                .with_year(start.year() + 1)
                .expect("year+1 stays in range"),
        ),
        7 => Some(start + chrono::Months::new(1)),
        10 => Some(start + chrono::Duration::days(1)),
        _ => None,
    }
}

/// The date filter document for one search value (#519). A free function so
/// the period semantics are unit-testable without a live MongoDB.
///
/// A partial date names a *period*, not an instant (mirroring the SQLite
/// mapping #463 pinned): `1995` is the whole year, `1995-10` the whole month,
/// `1995-10-02` the whole day. `eq` used to compare the exact start instant,
/// so month/year queries matched nothing.
fn build_date_filter_doc(value: &SearchValue, field: &str) -> StorageResult<Document> {
    let parsed = parse_date_for_query(&value.value).ok_or_else(|| {
        StorageError::Search(SearchError::QueryParseError {
            message: format!("Invalid date value '{}'", value.value),
        })
    })?;

    let end = implied_period_end(&value.value, parsed).map(chrono_to_bson);
    let start = chrono_to_bson(parsed);

    let filter = match (value.prefix, end) {
        (SearchPrefix::Ap, _) => {
            let lower = chrono_to_bson(parsed - chrono::Duration::hours(12));
            let upper = chrono_to_bson(parsed + chrono::Duration::hours(12));
            doc! { field: { "$gte": lower, "$lte": upper } }
        }
        (SearchPrefix::Eq, Some(end)) => doc! { field: { "$gte": start, "$lt": end } },
        (SearchPrefix::Eq, None) => doc! { field: { "$eq": start } },
        (SearchPrefix::Ne, Some(end)) => doc! {
            "$or": [
                { field: { "$lt": start } },
                { field: { "$gte": end } },
            ]
        },
        (SearchPrefix::Ne, None) => doc! { field: { "$ne": start } },
        // gt / sa: strictly after the whole period.
        (SearchPrefix::Gt | SearchPrefix::Sa, Some(end)) => doc! { field: { "$gte": end } },
        (SearchPrefix::Gt | SearchPrefix::Sa, None) => doc! { field: { "$gt": start } },
        // lt / eb: strictly before the whole period.
        (SearchPrefix::Lt | SearchPrefix::Eb, _) => doc! { field: { "$lt": start } },
        (SearchPrefix::Ge, _) => doc! { field: { "$gte": start } },
        (SearchPrefix::Le, Some(end)) => doc! { field: { "$lt": end } },
        (SearchPrefix::Le, None) => doc! { field: { "$lte": start } },
    };
    Ok(filter)
}

fn parse_date_for_query(value: &str) -> Option<DateTime<Utc>> {
    let normalized = if value.contains('T') {
        if value.contains('Z') || value.contains('+') || value.matches('-').count() > 2 {
            value.to_string()
        } else {
            format!("{}+00:00", value)
        }
    } else if value.len() == 10 {
        format!("{}T00:00:00+00:00", value)
    } else if value.len() == 7 {
        format!("{}-01T00:00:00+00:00", value)
    } else if value.len() == 4 {
        format!("{}-01-01T00:00:00+00:00", value)
    } else {
        value.to_string()
    };

    DateTime::parse_from_rfc3339(&normalized)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

async fn collect_documents(mut cursor: Cursor<Document>) -> StorageResult<Vec<Document>> {
    let mut docs = Vec::new();
    while cursor
        .advance()
        .await
        .or_query_error("Failed to advance MongoDB cursor")?
    {
        let doc = cursor
            .deserialize_current()
            .or_query_error("Failed to deserialize MongoDB document")?;
        docs.push(doc);
    }
    Ok(docs)
}

/// Orders `(resource_id, key)` pairs the way the server's
/// `{ key: <order>, _id: 1 }` sort would: by key in the requested direction,
/// ties broken by id ascending in both directions. Returns the ids.
fn order_sort_keys(
    mut keyed: Vec<(String, Bson)>,
    direction: crate::types::SortDirection,
) -> Vec<String> {
    use crate::types::SortDirection;
    keyed.sort_by(|(id_a, key_a), (id_b, key_b)| {
        let by_key = compare_sort_keys(key_a, key_b);
        let by_key = match direction {
            SortDirection::Ascending => by_key,
            SortDirection::Descending => by_key.reverse(),
        };
        by_key.then_with(|| id_a.cmp(id_b))
    });
    keyed.into_iter().map(|(id, _)| id).collect()
}

/// Total order over the sort-key values the search index writes: dates by
/// instant, strings byte-wise (the server's default, collation-free order),
/// numbers numerically across the integer/double representations. Mixed
/// types within one parameter do not occur (the writer emits one field per
/// parameter type); they fall back to a fixed type rank so the order is
/// still total.
fn compare_sort_keys(a: &Bson, b: &Bson) -> std::cmp::Ordering {
    match (a, b) {
        (Bson::DateTime(x), Bson::DateTime(y)) => x.cmp(y),
        (Bson::String(x), Bson::String(y)) => x.cmp(y),
        _ => match (sort_key_as_f64(a), sort_key_as_f64(b)) {
            (Some(x), Some(y)) => x.total_cmp(&y),
            _ => sort_key_type_rank(a)
                .cmp(&sort_key_type_rank(b))
                .then_with(|| a.to_string().cmp(&b.to_string())),
        },
    }
}

fn sort_key_as_f64(value: &Bson) -> Option<f64> {
    match value {
        Bson::Double(d) => Some(*d),
        Bson::Int32(i) => Some(f64::from(*i)),
        Bson::Int64(i) => Some(*i as f64),
        Bson::Decimal128(d) => d.to_string().parse().ok(),
        _ => None,
    }
}

fn sort_key_type_rank(value: &Bson) -> u8 {
    match value {
        Bson::Null => 0,
        Bson::Double(_) | Bson::Int32(_) | Bson::Int64(_) | Bson::Decimal128(_) => 1,
        Bson::String(_) => 2,
        Bson::DateTime(_) => 3,
        _ => 4,
    }
}

/// Finds the `contained[]` entry with the given local `id` in a container's
/// content.
fn extract_contained_resource(content: &Value, local_id: &str) -> Option<Value> {
    content
        .get("contained")?
        .as_array()?
        .iter()
        .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(local_id))
        .cloned()
}

/// Builds a `StoredResource` for a contained resource, inheriting the
/// container's version/tenant/timestamps. Used for `_containedType=contained`.
fn build_contained_stored(
    container: &StoredResource,
    contained_type: &str,
    local_id: &str,
    content: Value,
) -> StoredResource {
    StoredResource::from_storage(
        contained_type.to_string(),
        local_id.to_string(),
        container.version_id().to_string(),
        container.tenant_id().clone(),
        content,
        container.created_at(),
        container.last_modified(),
        None,
        container.fhir_version(),
    )
}

fn parse_simple_search_params(params: &str) -> Vec<(String, String)> {
    params
        .split('&')
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            Some((name.to_string(), value.to_string()))
        })
        .collect()
}

#[async_trait]
impl SearchProvider for MongoBackend {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        // `_contained` search uses a dedicated path (separate index rows and
        // heterogeneous result types); standard search handles `_contained=false`
        // (contained rows carry the container's resource_type, so the standard
        // type-scoped filter naturally excludes them).
        if query.contained != crate::types::ContainedMode::Off {
            return self.search_contained(tenant, query).await;
        }

        self.validate_query_support(query)?;

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let cursor = if let Some(cursor_str) = &query.cursor {
            Some(PageCursor::decode(cursor_str).map_err(|_| {
                StorageError::Search(SearchError::InvalidCursor {
                    cursor: cursor_str.clone(),
                })
            })?)
        } else {
            None
        };

        if cursor.is_some() && !query.sort.is_empty() {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message:
                    "MongoDB cursor pagination currently supports only default _lastUpdated sort"
                        .to_string(),
            }));
        }

        let previous_mode = cursor
            .as_ref()
            .is_some_and(|c| c.direction() == CursorDirection::Previous);

        let matched_ids = self
            .matching_resource_ids(&db, tenant_id, &query.resource_type, query)
            .await?;

        // Sorting by an indexed search parameter (#881): the sort key lives
        // in the search index, not on the resource documents, so the ordered
        // id list is computed there and the page fetched by id. Offset-paged;
        // cursor pagination with any custom sort is already rejected above.
        let param_sort = query
            .sort
            .iter()
            .find(|d| !matches!(d.parameter.as_str(), "_id" | "_lastUpdated" | "_score"));
        if let Some(directive) = param_sort {
            if query.sort.len() > 1 {
                return Err(StorageError::Search(SearchError::QueryParseError {
                    message: "MongoDB supports a single _sort directive when sorting by a                               search parameter"
                        .to_string(),
                }));
            }
            return self
                .search_param_sorted(tenant, query, &db, tenant_id, matched_ids, directive)
                .await;
        }

        let filter = self.build_resource_filter(
            tenant_id,
            &query.resource_type,
            query,
            matched_ids.as_ref(),
            cursor.as_ref(),
        )?;

        let sort = self.build_sort_document(query, previous_mode)?;
        let page_size = query.count.unwrap_or(100).max(1) as usize;

        let mut find_action = resources
            .find(filter)
            .sort(sort)
            .limit((page_size + 1) as i64);

        if cursor.is_none() {
            if let Some(offset) = query.offset {
                find_action = find_action.skip(offset as u64);
            }
        }

        let docs = collect_documents(
            find_action
                .await
                .or_query_error("Failed to execute MongoDB search")?,
        )
        .await?;

        let mut resources = docs
            .into_iter()
            .map(|doc| self.document_to_stored_resource(tenant, &query.resource_type, doc))
            .collect::<StorageResult<Vec<_>>>()?;

        if previous_mode {
            resources.reverse();
        }

        let has_next = resources.len() > page_size;
        if has_next {
            let _ = resources.pop();
        }

        let has_previous = cursor.is_some() || query.offset.unwrap_or(0) > 0;

        let next_cursor = if has_next {
            resources.last().map(|resource| {
                PageCursor::new(
                    vec![CursorValue::String(resource.last_modified().to_rfc3339())],
                    resource.id(),
                )
                .encode()
            })
        } else {
            None
        };

        let previous_cursor = if has_previous {
            resources.first().map(|resource| {
                PageCursor::previous(
                    vec![CursorValue::String(resource.last_modified().to_rfc3339())],
                    resource.id(),
                )
                .encode()
            })
        } else {
            None
        };

        let total = if query.total.is_some() {
            Some(self.search_count(tenant, query).await?)
        } else {
            None
        };

        let page_info = PageInfo {
            next_cursor,
            previous_cursor,
            total,
            has_next,
            has_previous,
        };

        let page = Page::new(resources, page_info);
        let mut included: Vec<StoredResource> = Vec::new();

        if !query.includes.is_empty() {
            let forward: Vec<IncludeDirective> = query
                .includes
                .iter()
                .filter(|i| i.include_type == IncludeType::Include)
                .cloned()
                .collect();
            if !forward.is_empty() {
                let resolved = self
                    .resolve_forward_includes_capped(tenant, &page.items, &forward)
                    .await?;
                Self::merge_unique(&mut included, resolved);
            }

            let reverse: Vec<IncludeDirective> = query
                .includes
                .iter()
                .filter(|i| i.include_type == IncludeType::Revinclude)
                .cloned()
                .collect();
            if !reverse.is_empty() {
                let resolved = self
                    .resolve_revincludes(tenant, &page.items, &reverse)
                    .await?;
                Self::merge_unique(&mut included, resolved);
            }
        }

        Ok(SearchResult {
            resources: page,
            included,
            total,
            scores: Default::default(),
        })
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        self.validate_query_support(query)?;

        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let matched_ids = self
            .matching_resource_ids(&db, tenant_id, &query.resource_type, query)
            .await?;

        let filter = self.build_resource_filter(
            tenant_id,
            &query.resource_type,
            query,
            matched_ids.as_ref(),
            None,
        )?;

        resources
            .count_documents(filter)
            .await
            .or_query_error("Failed to count MongoDB search results")
    }

    fn search_param_registry(
        &self,
        tenant: &crate::tenant::TenantContext,
    ) -> std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        self.tenant_registry(tenant.tenant_id().as_str())
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

#[async_trait]
impl ConditionalStorage for MongoBackend {
    async fn conditional_create(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalCreateResult> {
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                let created = self
                    .create(tenant, resource_type, resource, fhir_version)
                    .await?;
                Ok(ConditionalCreateResult::Created(created))
            }
            1 => Ok(ConditionalCreateResult::Exists(
                matches.into_iter().next().expect("single match must exist"),
            )),
            n => Ok(ConditionalCreateResult::MultipleMatches(n)),
        }
    }

    async fn conditional_update(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        resource: Value,
        search_params: &str,
        upsert: bool,
        fhir_version: FhirVersion,
    ) -> StorageResult<ConditionalUpdateResult> {
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => {
                if upsert {
                    let created = self
                        .create(tenant, resource_type, resource, fhir_version)
                        .await?;
                    Ok(ConditionalUpdateResult::Created(created))
                } else {
                    Ok(ConditionalUpdateResult::NoMatch)
                }
            }
            1 => {
                let current = matches.into_iter().next().expect("single match must exist");
                let updated = self.update(tenant, &current, resource).await?;
                Ok(ConditionalUpdateResult::Updated(updated))
            }
            n => Ok(ConditionalUpdateResult::MultipleMatches(n)),
        }
    }

    async fn conditional_delete(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
    ) -> StorageResult<ConditionalDeleteResult> {
        let matches = self
            .find_matching_resources(tenant, resource_type, search_params)
            .await?;

        match matches.len() {
            0 => Ok(ConditionalDeleteResult::NoMatch),
            1 => {
                let current = matches.into_iter().next().expect("single match must exist");
                self.delete(tenant, resource_type, current.id()).await?;
                Ok(ConditionalDeleteResult::Deleted(current))
            }
            n => Ok(ConditionalDeleteResult::MultipleMatches(n)),
        }
    }

    async fn conditional_patch(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params: &str,
        patch: &PatchFormat,
    ) -> StorageResult<ConditionalPatchResult> {
        let _ = (tenant, resource_type, search_params, patch);
        Err(StorageError::Backend(BackendError::UnsupportedCapability {
            backend_name: "mongodb".to_string(),
            capability: "conditional_patch".to_string(),
        }))
    }
}

impl MongoBackend {
    /// Executes a `_contained=true|both` search (see the SQLite backend's
    /// `search_contained` for shared semantics). Returns containers (default) or
    /// contained resources (`_containedType=contained`); `both` merges top-level
    /// matches first. Single window (no keyset cursor).
    async fn search_contained(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        use crate::types::{ContainedMode, ContainedReturn};

        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let contained_type = query.resource_type.as_str();

        let matches = self
            .matching_contained(&db, tenant_id, contained_type, query)
            .await?;

        let mut items: Vec<StoredResource> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        match query.contained_return {
            ContainedReturn::Container => {
                for (ctype, cid, _) in &matches {
                    if !seen.insert(format!("{ctype}/{cid}")) {
                        continue;
                    }
                    if let Some(container) = self.read(tenant, ctype, cid).await? {
                        items.push(container);
                    }
                }
            }
            ContainedReturn::Contained => {
                for (ctype, cid, local) in &matches {
                    let Some(local_id) = local else { continue };
                    if !seen.insert(format!("{ctype}/{cid}#{local_id}")) {
                        continue;
                    }
                    if let Some(container) = self.read(tenant, ctype, cid).await? {
                        if let Some(c) = extract_contained_resource(container.content(), local_id) {
                            items.push(build_contained_stored(
                                &container,
                                contained_type,
                                local_id,
                                c,
                            ));
                        }
                    }
                }
            }
        }

        if query.contained == ContainedMode::Both {
            let mut top_query = query.clone();
            top_query.contained = ContainedMode::Off;
            top_query.contained_return = ContainedReturn::Container;
            let top = self.search(tenant, &top_query).await?;
            let mut merged = top.resources.items;
            let top_urls: HashSet<String> = merged.iter().map(|r| r.url()).collect();
            for item in items {
                if !top_urls.contains(&item.url()) {
                    merged.push(item);
                }
            }
            items = merged;
        }

        let count = query.count.unwrap_or(100) as usize;
        let offset = query.offset.unwrap_or(0) as usize;
        let total = if query.total.is_some() {
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

    /// Resolves `_contained` matches via an aggregation over `search_index`,
    /// grouping contained rows by the contained entity
    /// `(resource_type, resource_id, contained_local_id)` and requiring every
    /// searched parameter to be present on that entity. Returns
    /// `(container_type, container_id, contained_local_id)` tuples.
    async fn matching_contained(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        contained_type: &str,
        query: &SearchQuery,
    ) -> StorageResult<Vec<(String, String, Option<String>)>> {
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);

        let mut branches: Vec<Bson> = Vec::new();
        let mut distinct_names: Vec<String> = Vec::new();
        for param in &query.parameters {
            if param.name.starts_with('_')
                || matches!(
                    param.param_type,
                    SearchParamType::Composite | SearchParamType::Special
                )
            {
                continue;
            }
            // Reuse the standard per-param value filter, dropping the tenant /
            // resource_type scoping (handled by the pipeline's top `$match`).
            let mut branch = self.build_search_index_filter("", "", param)?;
            branch.remove("tenant_id");
            branch.remove("resource_type");
            branches.push(Bson::Document(branch));
            if !distinct_names.contains(&param.name) {
                distinct_names.push(param.name.clone());
            }
        }
        if branches.is_empty() {
            return Ok(Vec::new());
        }

        let mut pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "is_contained": true,
                "contained_type": contained_type,
                "$or": branches,
            }},
            doc! { "$group": {
                "_id": {
                    "rtype": "$resource_type",
                    "rid": "$resource_id",
                    "lid": "$contained_local_id",
                },
                "names": { "$addToSet": "$param_name" },
            }},
        ];
        if distinct_names.len() > 1 {
            pipeline.push(doc! { "$match": { "names": { "$all": distinct_names } } });
        }

        let cursor = search_index
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to aggregate contained search")?;
        let docs = collect_documents(cursor).await?;

        let mut out = Vec::new();
        for doc in docs {
            if let Ok(id) = doc.get_document("_id") {
                let rtype = id.get_str("rtype").unwrap_or_default().to_string();
                let rid = id.get_str("rid").unwrap_or_default().to_string();
                let lid = id.get_str("lid").ok().map(ToString::to_string);
                if !rtype.is_empty() && !rid.is_empty() {
                    out.push((rtype, rid, lid));
                }
            }
        }
        Ok(out)
    }

    fn validate_query_support(&self, query: &SearchQuery) -> StorageResult<()> {
        if query.parameters.iter().any(|param| !param.chain.is_empty()) {
            return Err(StorageError::Search(
                SearchError::ChainedSearchNotSupported {
                    chain: "forward chain".to_string(),
                },
            ));
        }

        if !query.reverse_chains.is_empty() {
            return Err(StorageError::Search(SearchError::ReverseChainNotSupported));
        }

        for param in &query.parameters {
            if matches!(
                param.modifier,
                Some(SearchModifier::Above)
                    | Some(SearchModifier::Below)
                    | Some(SearchModifier::In)
                    | Some(SearchModifier::NotIn)
            ) {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: param
                        .modifier
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    param_type: param.param_type.to_string(),
                }));
            }

            // `_id`/`_lastUpdated` are lowered outside the generic modifier
            // dispatch (see `build_resource_id_condition` /
            // `build_resource_last_updated_conditions`), which only knows
            // how to honour a narrow set of modifiers on them. Reject
            // anything else explicitly here rather than letting it fall
            // through to a builder that would silently misinterpret it
            // (#1055).
            let metadata_param_honoured = match param.name.as_str() {
                "_id" => matches!(
                    param.modifier,
                    None | Some(SearchModifier::Not) | Some(SearchModifier::Missing)
                ),
                "_lastUpdated" => {
                    matches!(param.modifier, None | Some(SearchModifier::Missing))
                }
                _ => true,
            };
            if !metadata_param_honoured {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: param
                        .modifier
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    param_type: param.param_type.to_string(),
                }));
            }
        }

        Ok(())
    }

    /// Search with `_sort` on an indexed parameter (#881): pages over the id
    /// ordering computed in the search index, then fetches the page by id and
    /// restores the order. Offset-paginated; no page cursors are issued —
    /// cursor pagination with a custom sort is rejected at the entry point.
    ///
    /// The rows, `has_next` and `total` of one page all derive from a single
    /// id sequence: the ordering, narrowed by the search-index matches and by
    /// the resource-level predicates (`_id`, `_lastUpdated`, live-only) that
    /// `resource_level_ids` resolves (#1056).
    async fn search_param_sorted(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
        db: &mongodb::Database,
        tenant_id: &str,
        matched_ids: Option<HashSet<String>>,
        directive: &crate::types::SortDirective,
    ) -> StorageResult<SearchResult> {
        // Any filtered sort resolves its candidate set through the resources
        // collection: that folds in the resource-level predicates (`_id`,
        // `_lastUpdated`) AND makes the set live-only by construction, so a
        // stale search-index row can never occupy a page slot (#1056/#1040).
        // With no filter at all `allowed` stays `None` and the ordering is
        // computed over the whole type.
        let allowed: Option<HashSet<String>> = match matched_ids {
            Some(set) if set.is_empty() => Some(set),
            Some(set) => Some(
                self.resource_level_ids(db, tenant_id, &query.resource_type, query, Some(&set))
                    .await?,
            ),
            None if Self::has_resource_level_params(query) => Some(
                self.resource_level_ids(db, tenant_id, &query.resource_type, query, None)
                    .await?,
            ),
            None => None,
        };

        // Nothing can match: skip the ordering aggregation entirely.
        let ordered: Vec<String> = if allowed.as_ref().is_some_and(|set| set.is_empty()) {
            Vec::new()
        } else {
            self.param_sorted_ids(
                db,
                tenant_id,
                &query.resource_type,
                directive,
                allowed.as_ref(),
            )
            .await?
        };

        let page_size = query.count.unwrap_or(100).max(1) as usize;
        let offset = query.offset.unwrap_or(0) as usize;
        let has_previous = offset > 0;
        let mut page_ids: Vec<String> = ordered
            .iter()
            .skip(offset)
            .take(page_size + 1)
            .cloned()
            .collect();
        let has_next = page_ids.len() > page_size;
        page_ids.truncate(page_size);

        // Fetch the page's documents and restore the computed order.
        let id_set: HashSet<String> = page_ids.iter().cloned().collect();
        let filter = self.build_resource_filter(
            tenant_id,
            &query.resource_type,
            query,
            Some(&id_set),
            None,
        )?;
        let resources_coll = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let docs = collect_documents(
            resources_coll
                .find(filter)
                .await
                .or_query_error("Failed to execute MongoDB search")?,
        )
        .await?;
        let mut by_id: std::collections::HashMap<String, StoredResource> = docs
            .into_iter()
            .map(|doc| self.document_to_stored_resource(tenant, &query.resource_type, doc))
            .collect::<StorageResult<Vec<_>>>()?
            .into_iter()
            .map(|r| (r.id().to_string(), r))
            .collect();
        let resources: Vec<StoredResource> =
            page_ids.iter().filter_map(|id| by_id.remove(id)).collect();

        // `ordered` is exactly the sequence the page was cut from, so its
        // length is the total by construction — no second resolution.
        let total = if query.wants_total() {
            Some(ordered.len() as u64)
        } else {
            None
        };
        let page_info = PageInfo {
            next_cursor: None,
            previous_cursor: None,
            total,
            has_next,
            has_previous,
        };
        let page = Page::new(resources, page_info);

        let mut included: Vec<StoredResource> = Vec::new();
        if !query.includes.is_empty() {
            let forward: Vec<IncludeDirective> = query
                .includes
                .iter()
                .filter(|i| i.include_type == IncludeType::Include)
                .cloned()
                .collect();
            if !forward.is_empty() {
                let resolved = self
                    .resolve_forward_includes_capped(tenant, &page.items, &forward)
                    .await?;
                Self::merge_unique(&mut included, resolved);
            }
            let reverse: Vec<IncludeDirective> = query
                .includes
                .iter()
                .filter(|i| i.include_type == IncludeType::Revinclude)
                .cloned()
                .collect();
            if !reverse.is_empty() {
                let resolved = self
                    .resolve_revincludes(tenant, &page.items, &reverse)
                    .await?;
                Self::merge_unique(&mut included, resolved);
            }
        }

        Ok(SearchResult {
            resources: page,
            included,
            total,
            scores: Default::default(),
        })
    }

    /// True when the query carries a predicate that lives on the resource
    /// document rather than in the search index.
    fn has_resource_level_params(query: &SearchQuery) -> bool {
        query
            .parameters
            .iter()
            .any(|p| matches!(p.name.as_str(), "_id" | "_lastUpdated"))
    }

    /// Ids of the live resources that satisfy the resource-level predicates
    /// (`_id`, `_lastUpdated`) within `matched_ids` — also the liveness
    /// filter for every filtered parameter sort. This is the same predicate
    /// `search_count` counts, so a page cut from this set and its `total`
    /// agree by construction (#1056).
    async fn resource_level_ids(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        query: &SearchQuery,
        matched_ids: Option<&HashSet<String>>,
    ) -> StorageResult<HashSet<String>> {
        let filter =
            self.build_resource_filter(tenant_id, resource_type, query, matched_ids, None)?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let cursor = resources
            .find(filter)
            .projection(doc! { "_id": 0, "id": 1 })
            .await
            .or_query_error("Failed to resolve _id/_lastUpdated filters")?;
        let docs = collect_documents(cursor).await?;
        Ok(docs
            .into_iter()
            .filter_map(|d| d.get_str("id").ok().map(ToString::to_string))
            .collect())
    }

    /// Distinct `resource_id`s in the search index matching `filter`.
    async fn distinct_resource_ids(
        &self,
        search_index: &mongodb::Collection<Document>,
        filter: Document,
    ) -> StorageResult<HashSet<String>> {
        Ok(search_index
            .distinct("resource_id", filter)
            .await
            .or_query_error("Failed to query search_index")?
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect())
    }

    /// Every live resource id of the type — the universe `:missing=true` and
    /// `:not` complement against (#881).
    async fn all_resource_ids(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
    ) -> StorageResult<HashSet<String>> {
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        Ok(resources
            .distinct(
                "id",
                doc! {
                    "tenant_id": tenant_id,
                    "resource_type": resource_type,
                    "is_deleted": false,
                },
            )
            .await
            .or_query_error("Failed to enumerate resource ids")?
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect())
    }

    /// Resource ids ordered by an indexed search parameter's value (#881):
    /// grouped per resource in the search index taking the smallest value
    /// for ascending sorts and the largest for descending (the SQL backends'
    /// MIN/MAX), with resources that have no value for the parameter
    /// appended last in id order.
    ///
    /// With `allowed` — the ids the query matched — the aggregation is
    /// bounded to that set (`resource_id: {$in: chunk}`, hinted onto
    /// `idx_search_composite`), the per-resource keys are ordered client-side
    /// and the unkeyed tail is taken from `allowed` itself, so the cost is
    /// proportional to the result set rather than to the resource type
    /// (#1040). Without `allowed` (no filter at all) the ordering is still
    /// computed over the whole type.
    async fn param_sorted_ids(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        directive: &crate::types::SortDirective,
        allowed: Option<&HashSet<String>>,
    ) -> StorageResult<Vec<String>> {
        use crate::types::{SearchParamType as Spt, SortDirection};

        let value_field = match directive.param_type {
            Some(Spt::Date) => "value_date",
            Some(Spt::Number) => "value_number",
            // The writer stores quantities in `value_quantity_value`, not
            // `value_number`; mapping them to the latter made every quantity
            // sort degrade silently to id order (#1040).
            Some(Spt::Quantity) => "value_quantity_value",
            Some(Spt::Token) => "value_token_code",
            Some(Spt::Reference) => "value_reference",
            Some(Spt::Uri) => "value_uri",
            _ => "value_string",
        };
        let (accumulator, order) = match directive.direction {
            SortDirection::Ascending => ("$min", 1),
            SortDirection::Descending => ("$max", -1),
        };
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);

        if let Some(allowed) = allowed {
            // Bounded path (#1040): one `$match`+`$group` per chunk of the
            // candidate set, then order the keys client-side. Chunks are cut
            // from a sorted view of the set so the sequence of commands is
            // deterministic for a given query.
            let mut candidates: Vec<&String> = allowed.iter().collect();
            candidates.sort();
            let mut keyed: Vec<(String, Bson)> = Vec::with_capacity(allowed.len());
            for chunk in candidates.chunks(SORT_ID_CHUNK) {
                let ids: Vec<Bson> = chunk.iter().map(|id| Bson::String((*id).clone())).collect();
                let pipeline = vec![
                    doc! { "$match": {
                        "tenant_id": tenant_id,
                        "resource_type": resource_type,
                        "resource_id": { "$in": Bson::Array(ids) },
                        "param_name": &directive.parameter,
                        value_field: { "$ne": Bson::Null },
                    }},
                    doc! { "$group": {
                        "_id": "$resource_id",
                        "key": { accumulator: format!("${value_field}") },
                    }},
                ];
                let cursor = search_index
                    .aggregate(pipeline)
                    .hint(mongodb::options::Hint::Name(
                        "idx_search_composite".to_string(),
                    ))
                    .await
                    .or_query_error("Failed to sort by search parameter")?;
                for d in collect_documents(cursor).await? {
                    if let (Ok(id), Some(key)) = (d.get_str("_id"), d.get("key")) {
                        keyed.push((id.to_string(), key.clone()));
                    }
                }
            }
            let mut ordered = order_sort_keys(keyed, directive.direction);
            let keyed_set: HashSet<String> = ordered.iter().cloned().collect();
            let mut unkeyed: Vec<String> = allowed
                .iter()
                .filter(|id| !keyed_set.contains(*id))
                .cloned()
                .collect();
            unkeyed.sort();
            ordered.extend(unkeyed);
            // `allowed` is live-only by construction — `search_param_sorted`
            // resolves every filtered candidate set through the resources
            // collection with `is_deleted: false` — so no liveness pass is
            // needed here.
            return Ok(ordered);
        }

        // Unfiltered sort: the ordering is still computed over the whole
        // type. Unchanged from #881/#1056 apart from the value-field mapping.
        let pipeline = vec![
            doc! { "$match": {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "param_name": &directive.parameter,
                value_field: { "$ne": Bson::Null },
            }},
            doc! { "$group": {
                "_id": "$resource_id",
                "key": { accumulator: format!("${value_field}") },
            }},
            doc! { "$sort": { "key": order, "_id": 1 } },
            doc! { "$project": { "_id": 1 } },
        ];
        let cursor = search_index
            .aggregate(pipeline)
            .await
            .or_query_error("Failed to sort by search parameter")?;
        let docs = collect_documents(cursor).await?;
        let mut ordered: Vec<String> = docs
            .into_iter()
            .filter_map(|d| d.get_str("_id").ok().map(ToString::to_string))
            .collect();

        // A search-index row whose resource is gone (deleted, or a stale
        // entry) must not occupy a slot in the sequence: the page fetch would
        // drop it and the page would come back short (#1056).
        let live = self.all_resource_ids(db, tenant_id, resource_type).await?;
        ordered.retain(|id| live.contains(id));

        // Resources without a value for the parameter sort last.
        let keyed: HashSet<String> = ordered.iter().cloned().collect();
        let mut unkeyed: Vec<String> = live.into_iter().filter(|id| !keyed.contains(id)).collect();
        unkeyed.sort();
        ordered.extend(unkeyed);
        Ok(ordered)
    }

    async fn matching_resource_ids(
        &self,
        db: &mongodb::Database,
        tenant_id: &str,
        resource_type: &str,
        query: &SearchQuery,
    ) -> StorageResult<Option<HashSet<String>>> {
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let mut matched: Option<HashSet<String>> = None;

        for param in &query.parameters {
            if matches!(param.name.as_str(), "_id" | "_lastUpdated") {
                continue;
            }

            // `:missing` resolves from index-entry presence alone (#881),
            // exactly like the SQL backends: `missing=false` is the set of
            // resources with any entry for the parameter, `missing=true` its
            // complement over the type's live resources.
            let ids = if matches!(param.modifier, Some(SearchModifier::Missing)) {
                let wants_missing = param
                    .values
                    .first()
                    .map(|v| v.value == "true")
                    .unwrap_or(false);
                let with_entry = self
                    .distinct_resource_ids(
                        &search_index,
                        doc! {
                            "tenant_id": tenant_id,
                            "resource_type": resource_type,
                            "param_name": &param.name,
                        },
                    )
                    .await?;
                if wants_missing {
                    let all = self.all_resource_ids(db, tenant_id, resource_type).await?;
                    all.difference(&with_entry).cloned().collect::<HashSet<_>>()
                } else {
                    with_entry
                }
            } else if matches!(param.modifier, Some(SearchModifier::Not)) {
                // `:not` is the complement of the positive match, and per the
                // spec it includes resources with no value for the parameter
                // at all (#881).
                let mut positive = param.clone();
                positive.modifier = None;
                let filter = self.build_search_index_filter(tenant_id, resource_type, &positive)?;
                let matching = self.distinct_resource_ids(&search_index, filter).await?;
                let all = self.all_resource_ids(db, tenant_id, resource_type).await?;
                all.difference(&matching).cloned().collect::<HashSet<_>>()
            } else {
                let filter = self.build_search_index_filter(tenant_id, resource_type, param)?;
                self.distinct_resource_ids(&search_index, filter).await?
            };

            if ids.is_empty() {
                return Ok(Some(HashSet::new()));
            }

            matched = Some(match matched {
                Some(current) => current
                    .intersection(&ids)
                    .cloned()
                    .collect::<HashSet<String>>(),
                None => ids,
            });

            if matched.as_ref().is_some_and(|set| set.is_empty()) {
                return Ok(matched);
            }
        }

        // Compartment membership: a resource joins the compartment if it
        // references the compartment via ANY of the membership params (OR),
        // per the FHIR CompartmentDefinition. Computed as a single OR query
        // over the search_index and intersected with the parameter matches.
        if let Some(comp) = &query.compartment {
            if let Some(ids) = self
                .compartment_resource_ids(&search_index, tenant_id, resource_type, comp)
                .await?
            {
                if ids.is_empty() {
                    return Ok(Some(HashSet::new()));
                }
                matched = Some(match matched {
                    Some(current) => current
                        .intersection(&ids)
                        .cloned()
                        .collect::<HashSet<String>>(),
                    None => ids,
                });
            }
        }

        Ok(matched)
    }

    /// Returns the resource IDs that are members of the compartment described by
    /// `comp`: resources that reference `comp.reference` through ANY of the
    /// membership params (logical OR). Reference matching is version-agnostic
    /// (mirrors the reference handler): the stored reference must equal the base
    /// reference or carry a `/_history/<vid>` suffix.
    ///
    /// Returns `Ok(None)` when `comp` carries no params or no reference (no
    /// restriction to apply), otherwise `Ok(Some(ids))` (possibly empty).
    async fn compartment_resource_ids(
        &self,
        search_index: &mongodb::Collection<Document>,
        tenant_id: &str,
        resource_type: &str,
        comp: &CompartmentMembership,
    ) -> StorageResult<Option<HashSet<String>>> {
        if comp.params.is_empty() || comp.reference.is_empty() {
            return Ok(None);
        }

        let base = strip_reference_version(&comp.reference);
        let params: Vec<Bson> = comp.params.iter().cloned().map(Bson::String).collect();

        let filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "param_name": { "$in": Bson::Array(params) },
            "$or": [
                { "value_reference": &base },
                { "value_reference": { "$regex": format!("^{}/_history/", regex_escape(base)) } },
            ],
        };

        let ids = search_index
            .distinct("resource_id", filter)
            .await
            .or_query_error("Failed to query search_index")?
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect::<HashSet<_>>();

        Ok(Some(ids))
    }

    pub(super) fn build_search_index_filter(
        &self,
        tenant_id: &str,
        resource_type: &str,
        param: &SearchParameter,
    ) -> StorageResult<Document> {
        if param.values.is_empty() {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!("Search parameter '{}' has no values", param.name),
            }));
        }

        let mut filter = doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "param_name": &param.name,
        };

        let value_filters = param
            .values
            .iter()
            .map(|value| self.build_index_value_filter(param, value))
            .collect::<StorageResult<Vec<_>>>()?;

        if value_filters.len() == 1 {
            if let Some(single) = value_filters.into_iter().next() {
                for (key, value) in single {
                    filter.insert(key, value);
                }
            }
            return Ok(filter);
        }

        // #1062: FHIR comma-separated values are OR for every parameter type
        // (https://build.fhir.org/search.html#combining) — Date and Number
        // used to get `$and` here, which was not merely stricter but wrong
        // in a way that emptied the whole result. This filter is evaluated
        // against a single `search_index` document (one value of one
        // parameter, for one resource), so a disjoint AND can never be
        // satisfied by any row: `distinct_resource_ids` then returns
        // nothing, and `matching_resource_ids`'s empty-set short circuit
        // empties the *entire* search result, not just this parameter. The
        // AND semantics callers actually want is the *repeated*-parameter
        // form (`?date=ge2020&date=le2021`), which arrives as separate
        // `SearchParameter`s and is intersected in `matching_resource_ids` —
        // unaffected by this change. Behavior change: a range-shaped comma
        // list (`?date=ge2020,le2021`) widens from "in 2020-2021" to "any
        // date >= 2020 OR any date <= 2021"; use the repeated form above for
        // a closed range.
        filter.insert(
            "$or",
            Bson::Array(value_filters.into_iter().map(Bson::Document).collect()),
        );

        Ok(filter)
    }

    fn build_index_value_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> StorageResult<Document> {
        match param.name.as_str() {
            "_text" | "_content" => {
                return Err(StorageError::Search(SearchError::TextSearchNotAvailable));
            }
            "_id" | "_lastUpdated" => {
                return Err(StorageError::Search(SearchError::QueryParseError {
                    message: format!(
                        "Special parameter '{}' should be resolved against resources, not search_index",
                        param.name
                    ),
                }));
            }
            _ => {}
        }

        match param.param_type {
            SearchParamType::String => self.build_string_filter(param, value),
            SearchParamType::Token => self.build_token_filter(param, value),
            SearchParamType::Date => self.build_date_filter(value, "value_date"),
            SearchParamType::Number => self.build_number_filter(value),
            SearchParamType::Reference => self.build_reference_filter(param, value),
            SearchParamType::Uri => self.build_uri_filter(param, value),
            SearchParamType::Quantity => self.build_quantity_filter(value),
            SearchParamType::Composite => {
                Err(StorageError::Search(SearchError::InvalidComposite {
                    message: "Composite search is not supported in MongoDB Phase 4".to_string(),
                }))
            }
            SearchParamType::Special => Err(StorageError::Search(
                SearchError::UnsupportedParameterType {
                    param_type: format!("special parameter {}", param.name),
                },
            )),
        }
    }

    fn build_string_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> StorageResult<Document> {
        if value.prefix != SearchPrefix::Eq {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "Unsupported prefix '{}' for string parameter '{}'",
                    value.prefix, param.name
                ),
            }));
        }

        // `value_string` holds the value as written, so the insensitive
        // variants ask Mongo for a case-insensitive regex (`$options: "i"`)
        // rather than relying on a pre-lowercased index. `:exact` compares the
        // raw value, which is what makes it case-sensitive per the spec.
        match param.modifier.as_ref() {
            None => Ok(doc! {
                "value_string": {
                    "$regex": format!("^{}", regex_escape(&value.value)),
                    "$options": "i"
                }
            }),
            Some(SearchModifier::Exact) => Ok(doc! { "value_string": value.value.as_str() }),
            // `:text` on a string is a case-insensitive partial match,
            // implemented here as a substring match (same as `:contains`).
            Some(SearchModifier::Contains | SearchModifier::Text) => Ok(doc! {
                "value_string": {
                    "$regex": regex_escape(&value.value),
                    "$options": "i"
                }
            }),
            Some(other) => Err(StorageError::Search(SearchError::UnsupportedModifier {
                modifier: other.to_string(),
                param_type: "string".to_string(),
            })),
        }
    }

    fn build_token_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> StorageResult<Document> {
        if value.prefix != SearchPrefix::Eq {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "Unsupported prefix '{}' for token parameter '{}'",
                    value.prefix, param.name
                ),
            }));
        }

        match param.modifier.as_ref() {
            None => {}
            // `:text` (contains) and `:code-text` (starts-with) match the
            // token's display text (Coding.display / CodeableConcept.text).
            Some(m @ (SearchModifier::Text | SearchModifier::CodeText)) => {
                let escaped = regex_escape(&value.value);
                let regex = if *m == SearchModifier::CodeText {
                    format!("^{}", escaped)
                } else {
                    escaped
                };
                return Ok(doc! {
                    "value_token_display": { "$regex": regex, "$options": "i" }
                });
            }
            Some(other) => {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: other.to_string(),
                    param_type: "token".to_string(),
                }));
            }
        }

        if let Some((system, code)) = value.value.split_once('|') {
            if system.is_empty() {
                Ok(doc! { "value_token_code": code })
            } else if code.is_empty() {
                Ok(doc! { "value_token_system": system })
            } else {
                Ok(doc! {
                    "value_token_system": system,
                    "value_token_code": code,
                })
            }
        } else {
            Ok(doc! { "value_token_code": &value.value })
        }
    }

    fn build_reference_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> StorageResult<Document> {
        if value.prefix != SearchPrefix::Eq {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "Unsupported prefix '{}' for reference parameter '{}'",
                    value.prefix, param.name
                ),
            }));
        }

        // :contains - case-insensitive substring match on the stored reference.
        if matches!(param.modifier.as_ref(), Some(SearchModifier::Contains)) {
            return Ok(doc! {
                "value_reference": {
                    "$regex": regex_escape(&value.value),
                    "$options": "i"
                }
            });
        }

        // :text (contains) / :code-text (starts-with) match Reference.display.
        if matches!(
            param.modifier.as_ref(),
            Some(SearchModifier::Text | SearchModifier::CodeText)
        ) {
            let escaped = regex_escape(&value.value);
            let regex = if matches!(param.modifier.as_ref(), Some(SearchModifier::CodeText)) {
                format!("^{}", escaped)
            } else {
                escaped
            };
            return Ok(doc! {
                "value_reference_display": { "$regex": regex, "$options": "i" }
            });
        }

        if let Some(modifier) = &param.modifier {
            return Err(StorageError::Search(SearchError::UnsupportedModifier {
                modifier: modifier.to_string(),
                param_type: "reference".to_string(),
            }));
        }

        if value.value.contains('/') {
            return Ok(doc! { "value_reference": &value.value });
        }

        Ok(doc! {
            "$or": [
                { "value_reference": &value.value },
                {
                    "value_reference": {
                        "$regex": format!("/{}$", regex_escape(&value.value))
                    }
                }
            ]
        })
    }

    fn build_uri_filter(
        &self,
        param: &SearchParameter,
        value: &SearchValue,
    ) -> StorageResult<Document> {
        if value.prefix != SearchPrefix::Eq {
            return Err(StorageError::Search(SearchError::QueryParseError {
                message: format!(
                    "Unsupported prefix '{}' for uri parameter '{}'",
                    value.prefix, param.name
                ),
            }));
        }

        match param.modifier.as_ref() {
            None | Some(SearchModifier::Exact) => Ok(doc! { "value_uri": &value.value }),
            Some(SearchModifier::Contains) => Ok(doc! {
                "value_uri": {
                    "$regex": regex_escape(&value.value)
                }
            }),
            Some(other) => Err(StorageError::Search(SearchError::UnsupportedModifier {
                modifier: other.to_string(),
                param_type: "uri".to_string(),
            })),
        }
    }

    fn build_date_filter(&self, value: &SearchValue, field: &str) -> StorageResult<Document> {
        build_date_filter_doc(value, field)
    }

    /// Builds a MongoDB filter for a quantity parameter.
    ///
    /// Value form: `[prefix]number[|system|code]` (or the `number|code` shorthand).
    /// The comparison runs on `value_quantity_value`; an optional system/code
    /// further constrain `value_quantity_system` / `value_quantity_unit` (the
    /// extractor stores the quantity code under the unit field).
    fn build_quantity_filter(&self, value: &SearchValue) -> StorageResult<Document> {
        let parts: Vec<&str> = value.value.splitn(3, '|').collect();
        let parsed = parts[0].parse::<f64>().map_err(|e| {
            StorageError::Search(SearchError::QueryParseError {
                message: format!("Invalid quantity value '{}': {}", value.value, e),
            })
        })?;

        let value_condition = match value.prefix {
            SearchPrefix::Ap => {
                let delta = (parsed.abs() * 0.1).max(0.1);
                doc! { "$gte": parsed - delta, "$lte": parsed + delta }
            }
            _ => {
                let op = Self::prefix_to_mongo_operator(value.prefix)?;
                doc! { op: parsed }
            }
        };

        let mut filter = doc! { "value_quantity_value": value_condition };
        match parts.as_slice() {
            // number|system|code
            [_, system, code] => {
                if !system.is_empty() {
                    filter.insert("value_quantity_system", *system);
                }
                if !code.is_empty() {
                    filter.insert("value_quantity_unit", *code);
                }
            }
            // number|code shorthand
            [_, code] => {
                if !code.is_empty() {
                    filter.insert("value_quantity_unit", *code);
                }
            }
            _ => {}
        }

        Ok(filter)
    }

    fn build_number_filter(&self, value: &SearchValue) -> StorageResult<Document> {
        let parsed = value.value.parse::<f64>().map_err(|e| {
            StorageError::Search(SearchError::QueryParseError {
                message: format!("Invalid number value '{}': {}", value.value, e),
            })
        })?;

        match value.prefix {
            SearchPrefix::Ap => {
                let delta = (parsed.abs() * 0.1).max(0.1);
                Ok(doc! {
                    "value_number": {
                        "$gte": parsed - delta,
                        "$lte": parsed + delta,
                    }
                })
            }
            _ => {
                let op = Self::prefix_to_mongo_operator(value.prefix)?;
                Ok(doc! {
                    "value_number": {
                        op: parsed,
                    }
                })
            }
        }
    }

    fn prefix_to_mongo_operator(prefix: SearchPrefix) -> StorageResult<&'static str> {
        match prefix {
            SearchPrefix::Eq => Ok("$eq"),
            SearchPrefix::Ne => Ok("$ne"),
            SearchPrefix::Gt | SearchPrefix::Sa => Ok("$gt"),
            SearchPrefix::Lt | SearchPrefix::Eb => Ok("$lt"),
            SearchPrefix::Ge => Ok("$gte"),
            SearchPrefix::Le => Ok("$lte"),
            SearchPrefix::Ap => Ok("$eq"),
        }
    }

    pub(super) fn build_resource_filter(
        &self,
        tenant_id: &str,
        resource_type: &str,
        query: &SearchQuery,
        matched_ids: Option<&HashSet<String>>,
        cursor: Option<&PageCursor>,
    ) -> StorageResult<Document> {
        let mut conditions = vec![doc! {
            "tenant_id": tenant_id,
            "resource_type": resource_type,
            "is_deleted": false,
        }];

        if let Some(ids) = matched_ids {
            let id_values = ids.iter().cloned().map(Bson::String).collect::<Vec<_>>();
            conditions.push(doc! {
                "id": { "$in": Bson::Array(id_values) }
            });
        }

        for param in &query.parameters {
            match param.name.as_str() {
                "_id" => {
                    conditions.push(self.build_resource_id_condition(param)?);
                }
                "_lastUpdated" => {
                    conditions.extend(self.build_resource_last_updated_conditions(param)?);
                }
                _ => {}
            }
        }

        if let Some(cursor) = cursor {
            conditions.push(self.build_cursor_condition(cursor)?);
        }

        if conditions.len() == 1 {
            return Ok(conditions.remove(0));
        }

        Ok(doc! {
            "$and": Bson::Array(conditions.into_iter().map(Bson::Document).collect())
        })
    }

    fn build_resource_id_condition(&self, param: &SearchParameter) -> StorageResult<Document> {
        // `:missing` is a presence test on `id`, not a value to compare
        // against it — resolved before the values loop so the boolean
        // literal ("true"/"false") is never consumed as an id (#1055).
        if matches!(param.modifier, Some(SearchModifier::Missing)) {
            let wants_missing = param
                .values
                .first()
                .map(|v| v.value == "true")
                .unwrap_or(false);
            return Ok(if wants_missing {
                doc! { "id": Bson::Null }
            } else {
                doc! { "id": { "$ne": Bson::Null } }
            });
        }

        let negated = match &param.modifier {
            None => false,
            Some(SearchModifier::Not) => true,
            Some(other) => {
                return Err(StorageError::Search(SearchError::UnsupportedModifier {
                    modifier: other.to_string(),
                    param_type: param.param_type.to_string(),
                }));
            }
        };

        let mut ids = Vec::new();
        for value in &param.values {
            if value.prefix != SearchPrefix::Eq {
                return Err(StorageError::Search(SearchError::QueryParseError {
                    message: format!("Unsupported prefix '{}' for _id parameter", value.prefix),
                }));
            }
            ids.push(value.value.clone());
        }

        Ok(match (ids.len(), negated) {
            (1, false) => doc! { "id": ids.remove(0) },
            (1, true) => doc! { "id": { "$ne": ids.remove(0) } },
            (_, false) => doc! {
                "id": { "$in": Bson::Array(ids.into_iter().map(Bson::String).collect()) }
            },
            (_, true) => doc! {
                "id": { "$nin": Bson::Array(ids.into_iter().map(Bson::String).collect()) }
            },
        })
    }

    fn build_resource_last_updated_conditions(
        &self,
        param: &SearchParameter,
    ) -> StorageResult<Vec<Document>> {
        match &param.modifier {
            None => {
                let mut conditions = param
                    .values
                    .iter()
                    .map(|value| self.build_date_filter(value, "last_updated"))
                    .collect::<StorageResult<Vec<_>>>()?;

                // #1062: comma-separated `_lastUpdated` values are OR, the
                // same defect and fix as `build_search_index_filter` above
                // (see the comment there). Returning a single combined
                // document - instead of one document per value - keeps
                // `build_resource_filter` unchanged: it still `extend`s
                // whatever comes back into the top-level `$and`, so two
                // *separate* `_lastUpdated` parameters (the repeated form)
                // still AND.
                if conditions.len() <= 1 {
                    return Ok(conditions);
                }
                Ok(vec![doc! {
                    "$or": Bson::Array(conditions.drain(..).map(Bson::Document).collect()),
                }])
            }
            // Presence test on `last_updated`, mirroring `_id:missing` above.
            // Every live resource carries a `last_updated`, so `:missing=true`
            // is trivially empty and `:missing=false` trivially everything -
            // but the boolean literal must never reach `build_date_filter`
            // (#1055: it previously 400'd there as an unparseable date).
            Some(SearchModifier::Missing) => {
                let wants_missing = param
                    .values
                    .first()
                    .map(|v| v.value == "true")
                    .unwrap_or(false);
                Ok(vec![if wants_missing {
                    doc! { "last_updated": Bson::Null }
                } else {
                    doc! { "last_updated": { "$ne": Bson::Null } }
                }])
            }
            Some(other) => Err(StorageError::Search(SearchError::UnsupportedModifier {
                modifier: other.to_string(),
                param_type: param.param_type.to_string(),
            })),
        }
    }

    fn build_cursor_condition(&self, cursor: &PageCursor) -> StorageResult<Document> {
        let timestamp = match cursor.sort_values().first() {
            Some(CursorValue::String(value)) => DateTime::parse_from_rfc3339(value)
                .map_err(|_| {
                    StorageError::Search(SearchError::InvalidCursor {
                        cursor: cursor.encode(),
                    })
                })?
                .with_timezone(&Utc),
            _ => {
                return Err(StorageError::Search(SearchError::InvalidCursor {
                    cursor: cursor.encode(),
                }));
            }
        };

        let ts = chrono_to_bson(timestamp);
        let id = cursor.resource_id().to_string();

        if cursor.direction() == CursorDirection::Previous {
            Ok(doc! {
                "$or": [
                    { "last_updated": { "$gt": ts } },
                    { "last_updated": ts, "id": { "$gt": id } }
                ]
            })
        } else {
            Ok(doc! {
                "$or": [
                    { "last_updated": { "$lt": ts } },
                    { "last_updated": ts, "id": { "$lt": id } }
                ]
            })
        }
    }

    fn build_sort_document(
        &self,
        query: &SearchQuery,
        previous_mode: bool,
    ) -> StorageResult<Document> {
        if query.sort.is_empty() {
            return Ok(if previous_mode {
                doc! { "last_updated": 1_i32, "id": 1_i32 }
            } else {
                doc! { "last_updated": -1_i32, "id": -1_i32 }
            });
        }

        let mut sort = Document::new();

        for directive in &query.sort {
            let field = match directive.parameter.as_str() {
                "_lastUpdated" => "last_updated",
                "_id" | "id" => "id",
                other => {
                    return Err(StorageError::Search(
                        SearchError::UnsupportedParameterType {
                            param_type: format!("sort parameter '{}'", other),
                        },
                    ));
                }
            };

            let mut dir = if directive.direction == crate::types::SortDirection::Descending {
                -1_i32
            } else {
                1_i32
            };

            if previous_mode {
                dir = -dir;
            }

            sort.insert(field, dir);
        }

        if !sort.contains_key("id") {
            sort.insert("id", if previous_mode { 1_i32 } else { -1_i32 });
        }

        Ok(sort)
    }

    fn document_to_stored_resource(
        &self,
        tenant: &TenantContext,
        fallback_resource_type: &str,
        doc: Document,
    ) -> StorageResult<StoredResource> {
        let resource_type = doc
            .get_str("resource_type")
            .ok()
            .unwrap_or(fallback_resource_type)
            .to_string();

        let id = doc
            .get_str("id")
            .map_err(|e| internal_error(format!("Missing resource id in search result: {}", e)))?
            .to_string();

        let version_id = doc
            .get_str("version_id")
            .map_err(|e| internal_error(format!("Missing version_id in search result: {}", e)))?
            .to_string();

        let payload = doc.get_document("data").map_err(|e| {
            internal_error(format!("Missing resource payload in search result: {}", e))
        })?;

        let content = bson::from_bson::<Value>(Bson::Document(payload.clone())).map_err(|e| {
            serialization_error(format!("Failed to deserialize resource payload: {}", e))
        })?;

        let now = Utc::now();
        let created_at = doc
            .get_datetime("created_at")
            .map(bson_to_chrono)
            .unwrap_or(now);

        let last_updated = doc
            .get_datetime("last_updated")
            .map(bson_to_chrono)
            .unwrap_or(created_at);

        let deleted_at = match doc.get("deleted_at") {
            Some(Bson::DateTime(value)) => Some(bson_to_chrono(value)),
            _ => None,
        };

        let fhir_version = doc
            .get_str("fhir_version")
            .ok()
            .and_then(FhirVersion::from_storage)
            .unwrap_or_else(FhirVersion::default_enabled);

        Ok(StoredResource::from_storage(
            resource_type,
            id,
            version_id,
            tenant.tenant_id().clone(),
            content,
            created_at,
            last_updated,
            deleted_at,
            fhir_version,
        ))
    }

    async fn find_matching_resources(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        search_params_str: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        let parsed_params = parse_simple_search_params(search_params_str);

        if parsed_params.is_empty() {
            return Ok(Vec::new());
        }

        let search_params = self.build_search_parameters(tenant, resource_type, &parsed_params);

        let query = SearchQuery {
            resource_type: resource_type.to_string(),
            parameters: search_params,
            count: Some(1000),
            ..Default::default()
        };

        let result = <Self as SearchProvider>::search(self, tenant, &query).await?;
        Ok(result.resources.items)
    }

    pub(super) fn build_search_parameters(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        params: &[(String, String)],
    ) -> Vec<SearchParameter> {
        let registry_arc = self.tenant_registry(tenant.tenant_id().as_str());
        let registry = registry_arc.read();

        params
            .iter()
            .map(|(name, value)| {
                let values = vec![SearchValue::parse(value)];
                let param_type =
                    crate::search::resolve_param_type(&registry, resource_type, name, &values);

                SearchParameter {
                    name: name.clone(),
                    param_type,
                    modifier: None,
                    values,
                    chain: vec![],
                    components: vec![],
                }
            })
            .collect()
    }

    fn merge_unique(target: &mut Vec<StoredResource>, additions: Vec<StoredResource>) {
        let mut seen: HashSet<String> = target
            .iter()
            .map(|r| format!("{}/{}", r.resource_type(), r.id()))
            .collect();
        for resource in additions {
            let key = format!("{}/{}", resource.resource_type(), resource.id());
            if seen.insert(key) {
                target.push(resource);
            }
        }
    }

    /// Resolves forward `_include` directives for `search()`/`search_param_sorted()`,
    /// one directive at a time, capped at `self.config().max_included_resources`
    /// per directive.
    ///
    /// `search()` only resolves hop 1 of `_include` inline — `:iterate`
    /// continuation is the REST layer's job against
    /// [`crate::core::resolve_includes_iterate_continuation`] (#1063), so each
    /// directive is copied with `iterate: false` before delegating to
    /// [`IncludeProvider::resolve_includes`] and only its own hop-1 result is
    /// counted against the cap. The cap is applied per directive rather than as
    /// one shared budget so that an earlier directive exhausting its cap cannot
    /// starve a later, unrelated one (#1061) — this mirrors the per-directive
    /// cap the previous inline extractor enforced, and lives here (a `search()`
    /// contract) rather than on the `IncludeProvider` trait itself, which has no
    /// concept of a resource-count budget.
    async fn resolve_forward_includes_capped(
        &self,
        tenant: &TenantContext,
        matches: &[StoredResource],
        forward: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        let limit = self.config().max_included_resources.max(1);

        let mut included: Vec<StoredResource> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut truncated_labels: Vec<String> = Vec::new();

        for directive in forward {
            let hop1 = IncludeDirective {
                iterate: false,
                ..directive.clone()
            };
            let resolved = self
                .resolve_includes(tenant, matches, std::slice::from_ref(&hop1))
                .await?;

            let mut directive_count = 0usize;
            for resource in resolved {
                let key = format!("{}/{}", resource.resource_type(), resource.id());
                if seen.contains(&key) {
                    continue;
                }
                if directive_count >= limit {
                    truncated_labels.push(format!(
                        "_include={}:{}",
                        directive.source_type, directive.search_param
                    ));
                    break;
                }
                seen.insert(key);
                directive_count += 1;
                included.push(resource);
            }
        }

        if !truncated_labels.is_empty() {
            included.push(crate::core::include_truncation_outcome(
                tenant,
                matches
                    .first()
                    .map(|r| r.fhir_version())
                    .unwrap_or_else(FhirVersion::default_enabled),
                limit,
                &truncated_labels.join(","),
                "increase HFS_MONGODB_MAX_INCLUDED_RESOURCES to raise the limit",
            ));
        }

        Ok(included)
    }

    /// Streams the `resource_id`s in `search_index` matching `filter`, capped
    /// at `limit` distinct ids.
    ///
    /// Replaces an unbounded `Collection::distinct("resource_id", ..)`: a
    /// `distinct` reply is a single BSON document capped at 16 MiB by mongod,
    /// which a wide reverse-reference set can exceed (error 17217, ~372k
    /// 36-char ids) — reachable on realistic corpora (#1061). A driver-paged
    /// `find` cursor can be abandoned as soon as `limit` distinct ids are
    /// seen, so neither the wire reply nor our memory scales with the
    /// referring set's true size.
    ///
    /// No `sort`: `resource_id` is not a prefix of `idx_search_reference`
    /// (`tenant_id`, `resource_type`, `param_name`, `value_reference`), so an
    /// in-memory sort would itself risk MongoDB's 32 MiB sort limit. Ids come
    /// back in index order, so which ids survive a given truncation is stable
    /// for a fixed index state (not otherwise meaningful or API-specified).
    async fn referring_resource_ids(
        search_index: &mongodb::Collection<Document>,
        filter: Document,
        limit: usize,
    ) -> StorageResult<(Vec<String>, bool)> {
        let mut cursor = search_index
            .find(filter)
            .projection(doc! { "resource_id": 1_i32, "_id": 0_i32 })
            .batch_size(1000)
            .await
            .or_query_error("Failed to query search_index for revinclude")?;

        let mut seen: HashSet<String> = HashSet::new();
        let mut ids: Vec<String> = Vec::new();
        let mut truncated = false;

        while cursor
            .advance()
            .await
            .or_query_error("Failed to advance search_index cursor")?
        {
            let doc = cursor
                .deserialize_current()
                .or_query_error("Failed to deserialize search_index document")?;
            let Ok(resource_id) = doc.get_str("resource_id") else {
                continue;
            };
            if !seen.insert(resource_id.to_string()) {
                continue;
            }
            if ids.len() >= limit {
                truncated = true;
                break;
            }
            ids.push(resource_id.to_string());
        }

        Ok((ids, truncated))
    }
}

#[async_trait]
impl IncludeProvider for MongoBackend {
    /// Delegates to the shared, registry-driven resolver so `_include` (and
    /// `:iterate`) follows the same search-parameter definitions (with
    /// FHIRPath expression evaluation) used to build the index, instead of a
    /// backend-specific reference extractor that looked the search parameter
    /// up as a literal JSON field name and disagreed with it whenever the
    /// parameter name differed from the element (e.g.
    /// `Encounter:service-provider` -> `serviceProvider`).
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
impl RevincludeProvider for MongoBackend {
    async fn resolve_revincludes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        revincludes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        if resources.is_empty() || revincludes.is_empty() {
            return Ok(Vec::new());
        }

        let db = self.get_database().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let search_index = db.collection::<Document>(MongoBackend::SEARCH_INDEX_COLLECTION);
        let resources_collection = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        // See the matching comment in `resolve_includes`: per directive, not shared.
        let limit = self.config().max_included_resources.max(1);

        let mut included: Vec<StoredResource> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut truncated_labels: Vec<String> = Vec::new();

        for revinclude in revincludes {
            if revinclude.source_type.is_empty() {
                continue;
            }

            let mut reference_values: Vec<String> = Vec::with_capacity(resources.len() * 2);
            for resource in resources {
                reference_values.push(format!("{}/{}", resource.resource_type(), resource.id()));
                reference_values.push(resource.id().to_string());
            }
            reference_values.sort();
            reference_values.dedup();

            if reference_values.is_empty() {
                continue;
            }

            let bson_values: Vec<Bson> = reference_values.into_iter().map(Bson::String).collect();
            let index_filter = doc! {
                "tenant_id": tenant_id,
                "resource_type": &revinclude.source_type,
                "param_name": &revinclude.search_param,
                "value_reference": { "$in": Bson::Array(bson_values) },
            };

            // Bounded, streamed id resolution (#1061) — see
            // `referring_resource_ids`. `reference_values` above is already
            // bounded by the page; this is what previously was not.
            let (matching_ids, index_truncated) =
                Self::referring_resource_ids(&search_index, index_filter, limit).await?;

            if matching_ids.is_empty() {
                continue;
            }

            let id_bson: Vec<Bson> = matching_ids.into_iter().map(Bson::String).collect();
            let resource_filter = doc! {
                "tenant_id": tenant_id,
                "resource_type": &revinclude.source_type,
                "is_deleted": false,
                "id": { "$in": Bson::Array(id_bson) },
            };

            let docs = collect_documents(
                resources_collection
                    .find(resource_filter)
                    .await
                    .or_query_error("Failed to fetch revinclude resources")?,
            )
            .await?;

            let mut directive_count = 0usize;
            let mut directive_truncated = index_truncated;
            for doc in docs {
                let stored =
                    self.document_to_stored_resource(tenant, &revinclude.source_type, doc)?;
                let key = format!("{}/{}", stored.resource_type(), stored.id());
                if seen.contains(&key) {
                    continue;
                }
                if directive_count >= limit {
                    directive_truncated = true;
                    break;
                }
                seen.insert(key);
                directive_count += 1;
                included.push(stored);
            }

            if directive_truncated {
                truncated_labels.push(format!(
                    "_revinclude={}:{}",
                    revinclude.source_type, revinclude.search_param
                ));
            }
        }

        if !truncated_labels.is_empty() {
            included.push(crate::core::include_truncation_outcome(
                tenant,
                resources
                    .first()
                    .map(|r| r.fhir_version())
                    .unwrap_or_else(FhirVersion::default_enabled),
                limit,
                &truncated_labels.join(","),
                "increase HFS_MONGODB_MAX_INCLUDED_RESOURCES to raise the limit",
            ));
        }

        Ok(included)
    }
}

#[cfg(test)]
mod date_filter_tests {
    use super::*;

    fn filter(raw: &str) -> Document {
        build_date_filter_doc(&SearchValue::parse(raw), "value_date").expect("valid date")
    }

    fn bounds(d: &Document) -> &Document {
        d.get_document("value_date").expect("field doc")
    }

    /// #519: every prefix arm over the period a partial date names, pinned
    /// without a live MongoDB. `1995-10` spans [1995-10-01, 1995-11-01).
    #[test]
    fn partial_dates_compare_as_periods() {
        let oct = chrono_to_bson(parse_date_for_query("1995-10").unwrap());
        let nov = chrono_to_bson(parse_date_for_query("1995-11").unwrap());

        let eq = filter("1995-10");
        assert_eq!(bounds(&eq).get_datetime("$gte").unwrap(), &oct);
        assert_eq!(bounds(&eq).get_datetime("$lt").unwrap(), &nov);

        // ne: outside the period, either side.
        let ne = filter("ne1995-10");
        let arms = ne.get_array("$or").expect("$or");
        assert_eq!(arms.len(), 2);

        // gt/sa start at the period's end; le runs to it.
        assert_eq!(
            bounds(&filter("gt1995-10")).get_datetime("$gte").unwrap(),
            &nov
        );
        assert_eq!(
            bounds(&filter("sa1995-10")).get_datetime("$gte").unwrap(),
            &nov
        );
        assert_eq!(
            bounds(&filter("le1995-10")).get_datetime("$lt").unwrap(),
            &nov
        );

        // lt/eb and ge compare against the start.
        assert_eq!(
            bounds(&filter("lt1995-10")).get_datetime("$lt").unwrap(),
            &oct
        );
        assert_eq!(
            bounds(&filter("eb1995-10")).get_datetime("$lt").unwrap(),
            &oct
        );
        assert_eq!(
            bounds(&filter("ge1995-10")).get_datetime("$gte").unwrap(),
            &oct
        );
    }

    /// Year and day precisions derive their own period ends; a full timestamp
    /// is an instant and keeps exact comparison.
    #[test]
    fn precision_decides_the_period_end() {
        let y1996 = chrono_to_bson(parse_date_for_query("1996").unwrap());
        assert_eq!(bounds(&filter("1995")).get_datetime("$lt").unwrap(), &y1996);

        let oct3 = chrono_to_bson(parse_date_for_query("1995-10-03").unwrap());
        assert_eq!(
            bounds(&filter("1995-10-02")).get_datetime("$lt").unwrap(),
            &oct3
        );

        let instant = filter("eq1995-10-02T08:30:00Z");
        assert!(bounds(&instant).get_datetime("$eq").is_ok(), "{instant}");

        let ne_instant = filter("ne1995-10-02T08:30:00Z");
        assert!(bounds(&ne_instant).get_datetime("$ne").is_ok());
        assert!(
            bounds(&filter("gt1995-10-02T08:30:00Z"))
                .get_datetime("$gt")
                .is_ok()
        );
        assert!(
            bounds(&filter("le1995-10-02T08:30:00Z"))
                .get_datetime("$lte")
                .is_ok()
        );
    }

    /// ap: ±12h around the start, unchanged semantics.
    #[test]
    fn ap_keeps_the_twelve_hour_window() {
        let ap = filter("ap1995-10-02");
        assert!(bounds(&ap).get_datetime("$gte").is_ok());
        assert!(bounds(&ap).get_datetime("$lte").is_ok());
    }

    /// Garbage stays an error, not a silent full scan.
    #[test]
    fn invalid_dates_error() {
        assert!(build_date_filter_doc(&SearchValue::parse("not-a-date"), "value_date").is_err());
    }
}

#[cfg(test)]
mod query_support_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;

    /// `birthdate:missing=true` carries the value `true`, which is not a
    /// date — `:missing` must be accepted as presence-only (#881) rather
    /// than falling through to type-specific value parsing.
    #[test]
    fn missing_is_supported_without_type_specific_value_parsing() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: Some(SearchModifier::Missing),
            values: vec![SearchValue::eq("true")],
            chain: vec![],
            components: vec![],
        });

        assert!(backend.validate_query_support(&query).is_ok());
    }

    #[test]
    fn below_is_still_rejected() {
        let backend = MongoBackend::new(MongoBackendConfig::default()).unwrap();
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "identifier".to_string(),
            param_type: SearchParamType::Token,
            modifier: Some(SearchModifier::Below),
            values: vec![SearchValue::eq("http://example.org|abc")],
            chain: vec![],
            components: vec![],
        });

        let error = backend.validate_query_support(&query).unwrap_err();
        assert!(matches!(
            error,
            StorageError::Search(SearchError::UnsupportedModifier {
                ref modifier,
                ..
            }) if modifier == "below"
        ));
    }
}

/// #1062: comma-separated search values are OR per FHIR
/// (https://build.fhir.org/search.html#combining), for every parameter
/// type. These pins target the exact filter document `build_search_index_filter`
/// sends to `distinct_resource_ids` (:832), because *where* the predicate is
/// evaluated is what made the old `$and` catastrophic rather than merely
/// strict: a `search_index` document holds one value of one parameter for
/// one resource, so a disjoint `$and` can never be satisfied by any single
/// row. `distinct` then returns empty, `matching_resource_ids`'s empty-set
/// short circuit fires, and the *entire* search result is emptied — not
/// just the one parameter.
///
/// This is unrelated to the repeated-parameter form
/// (`?birthdate=ge1980-01-01&birthdate=lt1990-01-01`), which arrives as two
/// separate `SearchParameter` entries and is intersected across parameters
/// in `matching_resource_ids`; that AND is correct per spec and is guarded
/// here too, so a change that widens the comma-list join cannot silently
/// widen the repeated form as well.
#[cfg(test)]
mod value_list_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    #[test]
    fn comma_separated_date_values_are_ored() {
        let backend = backend();
        let param = SearchParameter {
            name: "birthdate".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::parse("2019"), SearchValue::parse("2021")],
            chain: vec![],
            components: vec![],
        };

        let filter = backend
            .build_search_index_filter("t1", "Patient", &param)
            .expect("valid filter");

        assert!(
            filter.get("$and").is_none(),
            "date value list must not be ANDed: {filter:?}"
        );
        let arms = filter.get_array("$or").expect("$or array");
        assert_eq!(arms.len(), 2);
        for arm in arms {
            let doc = arm.as_document().expect("$or arm is a document");
            assert!(
                doc.contains_key("value_date"),
                "each $or arm constrains value_date: {doc:?}"
            );
        }
    }

    /// A regression a partial fix could pass: dropping only `Date` from the
    /// old `matches!` would leave `Number` ANDed and this test red.
    #[test]
    fn comma_separated_number_values_are_ored() {
        let backend = backend();
        let param = SearchParameter {
            name: "factor-override".to_string(),
            param_type: SearchParamType::Number,
            modifier: None,
            values: vec![SearchValue::parse("0.25"), SearchValue::parse("1.5")],
            chain: vec![],
            components: vec![],
        };

        let filter = backend
            .build_search_index_filter("t1", "ChargeItem", &param)
            .expect("valid filter");

        assert!(
            filter.get("$and").is_none(),
            "number value list must not be ANDed: {filter:?}"
        );
        let arms = filter.get_array("$or").expect("$or array");
        assert_eq!(arms.len(), 2);
        for arm in arms {
            let doc = arm.as_document().expect("$or arm is a document");
            assert!(
                doc.contains_key("value_number"),
                "each $or arm constrains value_number: {doc:?}"
            );
        }
    }

    /// Quantity was never in the AND list. Confirmed here rather than
    /// assumed, so this module pins the join for every parameter type in
    /// one place.
    #[test]
    fn comma_separated_quantity_values_stay_ored() {
        let backend = backend();
        let param = SearchParameter {
            name: "value-quantity".to_string(),
            param_type: SearchParamType::Quantity,
            modifier: None,
            values: vec![SearchValue::parse("5"), SearchValue::parse("10")],
            chain: vec![],
            components: vec![],
        };

        let filter = backend
            .build_search_index_filter("t1", "Observation", &param)
            .expect("valid filter");

        assert!(filter.get("$and").is_none());
        assert_eq!(filter.get_array("$or").expect("$or array").len(), 2);
    }

    /// Second site, same defect, one function away:
    /// `build_resource_last_updated_conditions` (:1508) is the `_lastUpdated`
    /// equivalent of `build_search_index_filter`, resolved against the
    /// resource document instead of `search_index`. A disjoint comma list
    /// must OR into a single combined condition.
    #[test]
    fn comma_separated_last_updated_values_are_ored() {
        let backend = backend();
        let query = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::parse("2019"), SearchValue::parse("2021")],
            chain: vec![],
            components: vec![],
        });

        let filter = backend
            .build_resource_filter("t1", "Patient", &query, None, None)
            .expect("valid filter");

        let and_arms = filter.get_array("$and").expect("$and array");
        assert_eq!(
            and_arms.len(),
            2,
            "base document plus one combined last_updated condition: {filter:?}"
        );
        let combined = and_arms[1].as_document().expect("condition is a document");
        let or_arms = combined.get_array("$or").expect("$or array");
        assert_eq!(or_arms.len(), 2);
        for arm in or_arms {
            let doc = arm.as_document().expect("$or arm is a document");
            assert!(doc.contains_key("last_updated"));
        }
    }

    /// Guard: the repeated-parameter form is a different mechanism (two
    /// separate `SearchParameter` entries, not one with two values) and
    /// must keep ANDing exactly as before — MANUAL_TESTING_MATRIX row 4.3
    /// depends on it. Must stay green before and after the fix.
    #[test]
    fn repeated_last_updated_parameters_still_and() {
        let backend = backend();
        let query = SearchQuery::new("Patient")
            .with_parameter(SearchParameter {
                name: "_lastUpdated".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::parse("ge2019")],
                chain: vec![],
                components: vec![],
            })
            .with_parameter(SearchParameter {
                name: "_lastUpdated".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::parse("le2021")],
                chain: vec![],
                components: vec![],
            });

        let filter = backend
            .build_resource_filter("t1", "Patient", &query, None, None)
            .expect("valid filter");

        let and_arms = filter.get_array("$and").expect("$and array");
        assert_eq!(
            and_arms.len(),
            3,
            "base document plus one condition per repeated parameter: {filter:?}"
        );
        for arm in &and_arms[1..] {
            let doc = arm.as_document().expect("condition is a document");
            assert!(doc.contains_key("last_updated"));
            assert!(
                doc.get("$or").is_none(),
                "a single-value condition stays flat, not wrapped in $or"
            );
        }
    }
}

/// #1055: `_id`/`_lastUpdated` are lowered outside the generic modifier
/// dispatch (`matching_resource_ids` skips them and hands them to
/// `build_resource_id_condition` / `build_resource_last_updated_conditions`
/// instead), so these two builders must honour `param.modifier` themselves.
/// Pinned at the filter-shape level, without a live MongoDB, because the
/// integration test cannot fully discriminate `_id:missing=true` (an
/// accidental pass either way — see the sibling integration test).
#[cfg(test)]
mod metadata_param_modifier_tests {
    use super::*;
    use crate::backends::mongodb::MongoBackendConfig;

    fn backend() -> MongoBackend {
        MongoBackend::new(MongoBackendConfig::default()).unwrap()
    }

    fn id_param(modifier: Option<SearchModifier>, values: &[&str]) -> SearchParameter {
        SearchParameter {
            name: "_id".to_string(),
            param_type: SearchParamType::Token,
            modifier,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            chain: vec![],
            components: vec![],
        }
    }

    fn last_updated_param(modifier: Option<SearchModifier>, value: &str) -> SearchParameter {
        SearchParameter {
            name: "_lastUpdated".to_string(),
            param_type: SearchParamType::Date,
            modifier,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        }
    }

    /// The metadata-param condition alone: `build_resource_filter`'s first
    /// `$and` arm is always the tenant/resource_type/is_deleted seed, so with
    /// a single parameter and no matched-id set the second arm is the
    /// condition under test.
    fn condition(backend: &MongoBackend, param: SearchParameter) -> Document {
        let query = SearchQuery::new("Patient").with_parameter(param);
        let filter = backend
            .build_resource_filter("t", "Patient", &query, None, None)
            .expect("filter should build");
        filter.get_array("$and").expect("$and array")[1]
            .as_document()
            .expect("condition document")
            .clone()
    }

    #[test]
    fn plain_id_match_is_unchanged() {
        let backend = backend();
        let cond = condition(&backend, id_param(None, &["pat-a"]));
        assert_eq!(cond.get_str("id").unwrap(), "pat-a");
    }

    #[test]
    fn id_not_negates_instead_of_matching() {
        let backend = backend();

        let single = condition(&backend, id_param(Some(SearchModifier::Not), &["pat-a"]));
        assert_eq!(
            single.get_document("id").unwrap().get_str("$ne").unwrap(),
            "pat-a"
        );

        let multi = condition(
            &backend,
            id_param(Some(SearchModifier::Not), &["pat-a", "pat-b"]),
        );
        let nin = multi.get_document("id").unwrap().get_array("$nin").unwrap();
        let values: Vec<&str> = nin.iter().map(|b| b.as_str().unwrap()).collect();
        assert_eq!(values, vec!["pat-a", "pat-b"]);
    }

    #[test]
    fn id_missing_is_a_presence_test_on_the_id_field() {
        let backend = backend();

        let missing_true = condition(&backend, id_param(Some(SearchModifier::Missing), &["true"]));
        assert!(matches!(missing_true.get("id"), Some(Bson::Null)));

        let missing_false = condition(
            &backend,
            id_param(Some(SearchModifier::Missing), &["false"]),
        );
        assert!(matches!(
            missing_false.get_document("id").unwrap().get("$ne"),
            Some(Bson::Null)
        ));
    }

    /// Control for #519: the refactor of `build_resource_last_updated_conditions`
    /// into a modifier match must leave the no-modifier (period-comparison) arm
    /// byte-identical.
    #[test]
    fn last_updated_no_modifier_control_still_builds_a_date_range() {
        let backend = backend();
        let cond = condition(&backend, last_updated_param(None, "2024"));
        let bounds = cond.get_document("last_updated").expect("field doc");
        assert!(bounds.get_datetime("$gte").is_ok());
        assert!(bounds.get_datetime("$lt").is_ok());
    }

    #[test]
    fn last_updated_missing_does_not_parse_the_boolean_as_a_date() {
        let backend = backend();

        let missing_true = condition(
            &backend,
            last_updated_param(Some(SearchModifier::Missing), "true"),
        );
        assert!(matches!(missing_true.get("last_updated"), Some(Bson::Null)));

        let missing_false = condition(
            &backend,
            last_updated_param(Some(SearchModifier::Missing), "false"),
        );
        assert!(matches!(
            missing_false
                .get_document("last_updated")
                .unwrap()
                .get("$ne"),
            Some(Bson::Null)
        ));
    }

    #[test]
    fn unhonoured_modifiers_on_metadata_params_are_rejected() {
        let backend = backend();

        let id_text = SearchQuery::new("Patient")
            .with_parameter(id_param(Some(SearchModifier::Text), &["abc"]));
        let err = backend.validate_query_support(&id_text).unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                if modifier == "text"
        ));

        let last_updated_not = SearchQuery::new("Patient")
            .with_parameter(last_updated_param(Some(SearchModifier::Not), "2024"));
        let err = backend
            .validate_query_support(&last_updated_not)
            .unwrap_err();
        assert!(matches!(
            err,
            StorageError::Search(SearchError::UnsupportedModifier { ref modifier, .. })
                if modifier == "not"
        ));

        // The three honoured combinations must not be rejected.
        assert!(
            backend
                .validate_query_support(
                    &SearchQuery::new("Patient")
                        .with_parameter(id_param(Some(SearchModifier::Not), &["pat-a"]))
                )
                .is_ok()
        );
        assert!(
            backend
                .validate_query_support(
                    &SearchQuery::new("Patient")
                        .with_parameter(id_param(Some(SearchModifier::Missing), &["true"]))
                )
                .is_ok()
        );
        assert!(
            backend
                .validate_query_support(
                    &SearchQuery::new("Patient")
                        .with_parameter(last_updated_param(Some(SearchModifier::Missing), "true"))
                )
                .is_ok()
        );
    }
}

/// #1040: `order_sort_keys` must reproduce the server's `{ key: <order>,
/// _id: 1 }` sort — including the tie-break — now that the bounded path
/// orders the keyed candidates client-side instead of server-side.
#[cfg(test)]
mod sort_key_order_tests {
    use super::*;
    use crate::types::SortDirection;

    fn dt(millis: i64) -> Bson {
        Bson::DateTime(mongodb::bson::DateTime::from_millis(millis))
    }

    #[test]
    fn dates_ascending_order_by_instant() {
        let keyed = vec![
            ("c".to_string(), dt(300)),
            ("a".to_string(), dt(100)),
            ("b".to_string(), dt(200)),
        ];
        assert_eq!(
            order_sort_keys(keyed, SortDirection::Ascending),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn dates_descending_reverses_by_instant_with_id_ascending_ties() {
        let keyed = vec![
            ("c".to_string(), dt(300)),
            ("a".to_string(), dt(100)),
            ("b".to_string(), dt(200)),
        ];
        assert_eq!(
            order_sort_keys(keyed.clone(), SortDirection::Descending),
            vec!["c", "b", "a"]
        );

        // Equal keys keep id-ascending order in BOTH directions — this is
        // the server's `{key: -1, _id: 1}` semantics.
        let tied = vec![("z".to_string(), dt(100)), ("y".to_string(), dt(100))];
        assert_eq!(
            order_sort_keys(tied.clone(), SortDirection::Ascending),
            vec!["y", "z"]
        );
        assert_eq!(
            order_sort_keys(tied, SortDirection::Descending),
            vec!["y", "z"]
        );
    }

    #[test]
    fn strings_order_byte_wise() {
        let keyed = vec![
            ("id-c".to_string(), Bson::String("c".to_string())),
            ("id-ba".to_string(), Bson::String("ba".to_string())),
            ("id-b".to_string(), Bson::String("b".to_string())),
        ];
        assert_eq!(
            order_sort_keys(keyed, SortDirection::Ascending),
            vec!["id-b", "id-ba", "id-c"]
        );

        let case_sensitive = vec![
            ("id-a".to_string(), Bson::String("a".to_string())),
            ("id-B".to_string(), Bson::String("B".to_string())),
        ];
        assert_eq!(
            order_sort_keys(case_sensitive, SortDirection::Ascending),
            vec!["id-B", "id-a"]
        );
    }

    #[test]
    fn numbers_order_numerically_across_representations() {
        let keyed = vec![
            ("id-7".to_string(), Bson::Int32(7)),
            ("id-6.5".to_string(), Bson::Double(6.5)),
            ("id-8".to_string(), Bson::Int64(8)),
        ];
        assert_eq!(
            order_sort_keys(keyed, SortDirection::Ascending),
            vec!["id-6.5", "id-7", "id-8"]
        );
    }
}
