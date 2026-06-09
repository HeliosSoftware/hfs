//! Search and conditional-operation implementation for MongoDB backend.

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
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
use crate::error::{BackendError, SearchError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    CompartmentMembership, CursorDirection, CursorValue, IncludeDirective, IncludeType, Page,
    PageCursor, PageInfo, SearchModifier, SearchParamType, SearchParameter, SearchPrefix,
    SearchQuery, SearchValue, StoredResource, strip_reference_version,
};

use super::MongoBackend;

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
        .map_err(|e| internal_error(format!("Failed to advance MongoDB cursor: {}", e)))?
    {
        let doc = cursor.deserialize_current().map_err(|e| {
            internal_error(format!("Failed to deserialize MongoDB document: {}", e))
        })?;
        docs.push(doc);
    }
    Ok(docs)
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
                .map_err(|e| internal_error(format!("Failed to execute MongoDB search: {}", e)))?,
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
                let resolved = self.resolve_includes(tenant, &page.items, &forward).await?;
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
            .map_err(|e| internal_error(format!("Failed to count MongoDB search results: {}", e)))
    }

    fn search_param_registry(
        &self,
    ) -> &std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        self.search_registry()
    }

    fn supports_contained_search(&self) -> bool {
        true
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
                Ok(ConditionalDeleteResult::Deleted)
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
            .map_err(|e| internal_error(format!("Failed to aggregate contained search: {}", e)))?;
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
        }

        Ok(())
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

            let filter = self.build_search_index_filter(tenant_id, resource_type, param)?;

            let ids = search_index
                .distinct("resource_id", filter)
                .await
                .map_err(|e| internal_error(format!("Failed to query search_index: {}", e)))?
                .into_iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect::<HashSet<_>>();

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
            .map_err(|e| internal_error(format!("Failed to query search_index: {}", e)))?
            .into_iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect::<HashSet<_>>();

        Ok(Some(ids))
    }

    fn build_search_index_filter(
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

        let combine_with_and = matches!(
            param.param_type,
            SearchParamType::Date | SearchParamType::Number
        );
        let operator = if combine_with_and { "$and" } else { "$or" };
        filter.insert(
            operator,
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

        let lowered = value.value.to_lowercase();
        match param.modifier.as_ref() {
            None => Ok(doc! {
                "value_string": {
                    "$regex": format!("^{}", regex_escape(&lowered))
                }
            }),
            Some(SearchModifier::Exact) => Ok(doc! { "value_string": lowered }),
            // `:text` on a string is a case-insensitive partial match,
            // implemented here as a substring match (same as `:contains`).
            Some(SearchModifier::Contains | SearchModifier::Text) => Ok(doc! {
                "value_string": {
                    "$regex": regex_escape(&lowered)
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
        let parsed = parse_date_for_query(&value.value).ok_or_else(|| {
            StorageError::Search(SearchError::QueryParseError {
                message: format!("Invalid date value '{}'", value.value),
            })
        })?;

        let bson_date = chrono_to_bson(parsed);

        match value.prefix {
            SearchPrefix::Ap => {
                let lower = chrono_to_bson(parsed - chrono::Duration::hours(12));
                let upper = chrono_to_bson(parsed + chrono::Duration::hours(12));
                Ok(doc! {
                    field: {
                        "$gte": lower,
                        "$lte": upper,
                    }
                })
            }
            _ => {
                let op = Self::prefix_to_mongo_operator(value.prefix)?;
                Ok(doc! {
                    field: {
                        op: bson_date,
                    }
                })
            }
        }
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

    fn build_resource_filter(
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
        let mut ids = Vec::new();

        for value in &param.values {
            if value.prefix != SearchPrefix::Eq {
                return Err(StorageError::Search(SearchError::QueryParseError {
                    message: format!("Unsupported prefix '{}' for _id parameter", value.prefix),
                }));
            }
            ids.push(value.value.clone());
        }

        if ids.len() == 1 {
            return Ok(doc! { "id": ids.remove(0) });
        }

        Ok(doc! {
            "id": { "$in": Bson::Array(ids.into_iter().map(Bson::String).collect()) }
        })
    }

    fn build_resource_last_updated_conditions(
        &self,
        param: &SearchParameter,
    ) -> StorageResult<Vec<Document>> {
        param
            .values
            .iter()
            .map(|value| self.build_date_filter(value, "last_updated"))
            .collect()
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

        let search_params = self.build_search_parameters(resource_type, &parsed_params);

        let query = SearchQuery {
            resource_type: resource_type.to_string(),
            parameters: search_params,
            count: Some(1000),
            ..Default::default()
        };

        let result = <Self as SearchProvider>::search(self, tenant, &query).await?;
        Ok(result.resources.items)
    }

    fn build_search_parameters(
        &self,
        resource_type: &str,
        params: &[(String, String)],
    ) -> Vec<SearchParameter> {
        let registry = self.search_registry().read();

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

    fn extract_references(content: &Value, search_param: &str) -> Vec<String> {
        let mut refs = Vec::new();
        if let Some(value) = content.get(search_param) {
            Self::collect_references_from_value(value, &mut refs);
        }
        refs
    }

    fn collect_references_from_value(value: &Value, refs: &mut Vec<String>) {
        match value {
            Value::Object(obj) => {
                if let Some(Value::String(reference)) = obj.get("reference") {
                    refs.push(reference.clone());
                }
                for v in obj.values() {
                    Self::collect_references_from_value(v, refs);
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    Self::collect_references_from_value(item, refs);
                }
            }
            _ => {}
        }
    }

    fn parse_reference(reference: &str) -> Option<(String, String)> {
        let trimmed = reference
            .strip_prefix("http://")
            .or_else(|| reference.strip_prefix("https://"))
            .unwrap_or(reference);

        let mut segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() < 2 {
            return None;
        }

        let id = segments.pop()?.to_string();
        let resource_type = segments.pop()?.to_string();

        if !resource_type
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        {
            return None;
        }

        Some((resource_type, id))
    }

    async fn fetch_resource_by_type_id(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let db = self.get_database().await?;
        let resources = db.collection::<Document>(MongoBackend::RESOURCES_COLLECTION);
        let tenant_id = tenant.tenant_id().as_str();

        let doc = resources
            .find_one(doc! {
                "tenant_id": tenant_id,
                "resource_type": resource_type,
                "id": id,
                "is_deleted": false,
            })
            .await
            .map_err(|e| internal_error(format!("Failed to fetch included resource: {}", e)))?;

        match doc {
            Some(doc) => Ok(Some(self.document_to_stored_resource(
                tenant,
                resource_type,
                doc,
            )?)),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl IncludeProvider for MongoBackend {
    async fn resolve_includes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        if resources.is_empty() || includes.is_empty() {
            return Ok(Vec::new());
        }

        let mut included = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for include in includes {
            for resource in resources {
                if resource.resource_type() != include.source_type {
                    continue;
                }

                let refs = Self::extract_references(resource.content(), &include.search_param);
                for reference in refs {
                    let Some((ref_type, ref_id)) = Self::parse_reference(&reference) else {
                        continue;
                    };

                    if let Some(target) = include.target_type.as_ref() {
                        if ref_type != *target {
                            continue;
                        }
                    }

                    let key = format!("{}/{}", ref_type, ref_id);
                    if !seen.insert(key) {
                        continue;
                    }

                    if let Some(stored) = self
                        .fetch_resource_by_type_id(tenant, &ref_type, &ref_id)
                        .await?
                    {
                        included.push(stored);
                    }
                }
            }
        }

        Ok(included)
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

        let mut included = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

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

            let matching_ids: Vec<String> = search_index
                .distinct("resource_id", index_filter)
                .await
                .map_err(|e| {
                    internal_error(format!(
                        "Failed to query search_index for revinclude: {}",
                        e
                    ))
                })?
                .into_iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect();

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

            let docs =
                collect_documents(resources_collection.find(resource_filter).await.map_err(
                    |e| internal_error(format!("Failed to fetch revinclude resources: {}", e)),
                )?)
                .await?;

            for doc in docs {
                let stored =
                    self.document_to_stored_resource(tenant, &revinclude.source_type, doc)?;
                let key = format!("{}/{}", stored.resource_type(), stored.id());
                if seen.insert(key) {
                    included.push(stored);
                }
            }
        }

        Ok(included)
    }
}
