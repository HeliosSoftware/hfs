//! Search implementation for PostgreSQL backend.
//!
//! This module provides search functionality for the PostgreSQL backend including:
//! - Basic single-type search
//! - Multi-type search
//! - _include and _revinclude support
//! - Chained search parameter support
//! - Full-text search using tsvector/tsquery

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;

use crate::core::{
    ChainedSearchProvider, IncludeProvider, MultiTypeSearchProvider, RevincludeProvider,
    SearchProvider, SearchResult, TextSearchProvider,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    CursorDirection, CursorValue, IncludeDirective, Page, PageCursor, PageInfo, Pagination,
    ReverseChainedParameter, SearchQuery, StoredResource,
};

use super::PostgresBackend;
use super::search::chain_builder::ChainQueryBuilder;
use super::search::query_builder::{PostgresQueryBuilder, SortValueKind, SqlParam};

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

#[async_trait]
impl SearchProvider for PostgresBackend {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = &query.resource_type;

        // Get count with default
        let count = query.count.unwrap_or(100) as usize;

        // Keyset key for cursor pagination. `None` for multi-field sorts, which
        // are returned as a single page rather than paged with an inconsistent
        // keyset.
        let keyset = PostgresQueryBuilder::primary_keyset_key(query);

        // Only honor an inbound cursor when we can build a keyset comparison.
        let cursor = if keyset.is_some() {
            query
                .cursor
                .as_ref()
                .and_then(|c| PageCursor::decode(c).ok())
        } else {
            None
        };

        // Param layout: $1=tenant, $2=type, then (cursor) $3=sort value, $4=id,
        // then the search-filter params.
        let param_offset = if cursor.is_some() { 4 } else { 2 };

        let search_filter = if !query.parameters.is_empty() {
            PostgresQueryBuilder::build_search_query(query, param_offset)
        } else {
            None
        };
        let filter_clause = search_filter
            .as_ref()
            .map(|f| format!(" AND ({})", f.sql))
            .unwrap_or_default();
        let search_params = search_filter.map(|f| f.params).unwrap_or_default();

        // SELECT the sort key alongside the row so the next cursor can be built.
        let select_cols = match &keyset {
            Some(k) => format!(
                "id, version_id, data, last_updated, fhir_version, {} AS sort_key",
                k.expr
            ),
            None => "id, version_id, data, last_updated, fhir_version".to_string(),
        };

        // ORDER BY for the first-page / offset paths.
        let order_by = if query.sort.is_empty() {
            "ORDER BY last_updated DESC, id ASC".to_string()
        } else {
            PostgresQueryBuilder::build_order_by(query)
        };

        // Build query based on pagination mode.
        let (sql, has_previous) = if let (Some(cursor), Some(k)) = (&cursor, &keyset) {
            let e = &k.expr;
            let asc = k.direction == crate::types::SortDirection::Ascending;
            match cursor.direction() {
                CursorDirection::Next => {
                    let e_op = if asc { ">" } else { "<" };
                    let sql = format!(
                        "SELECT {cols} FROM resources \
                         WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE{filter} \
                         AND ({e} {e_op} $3 OR ({e} = $3 AND id > $4)) \
                         ORDER BY {e} {dir}, id ASC LIMIT {lim}",
                        cols = select_cols,
                        filter = filter_clause,
                        e = e,
                        e_op = e_op,
                        dir = if asc { "ASC" } else { "DESC" },
                        lim = count + 1,
                    );
                    (sql, true)
                }
                CursorDirection::Previous => {
                    let e_op = if asc { "<" } else { ">" };
                    let sql = format!(
                        "SELECT {cols} FROM resources \
                         WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE{filter} \
                         AND ({e} {e_op} $3 OR ({e} = $3 AND id < $4)) \
                         ORDER BY {e} {dir}, id DESC LIMIT {lim}",
                        cols = select_cols,
                        filter = filter_clause,
                        e = e,
                        e_op = e_op,
                        dir = if asc { "DESC" } else { "ASC" },
                        lim = count + 1,
                    );
                    (sql, false)
                }
            }
        } else if let Some(offset) = query.offset {
            let sql = format!(
                "SELECT {cols} FROM resources \
                 WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE{filter} \
                 {order} LIMIT {lim} OFFSET {off}",
                cols = select_cols,
                filter = filter_clause,
                order = order_by,
                lim = count + 1,
                off = offset,
            );
            (sql, offset > 0)
        } else {
            let sql = format!(
                "SELECT {cols} FROM resources \
                 WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE{filter} \
                 {order} LIMIT {lim}",
                cols = select_cols,
                filter = filter_clause,
                order = order_by,
                lim = count + 1,
            );
            (sql, false)
        };

        // Build parameter list for binding.
        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];
        if let (Some(cursor), Some(k)) = (&cursor, &keyset) {
            Self::bind_cursor_value(&mut params, k.kind, cursor)?;
            params.push(Box::new(cursor.resource_id().to_string()));
        }
        for param in &search_params {
            match param {
                SqlParam::Text(s) => params.push(Box::new(s.clone())),
                SqlParam::Float(f) => params.push(Box::new(*f)),
                SqlParam::Integer(i) => params.push(Box::new(*i)),
                SqlParam::Bool(b) => params.push(Box::new(*b)),
                SqlParam::Timestamp(dt) => params.push(Box::new(*dt)),
                SqlParam::Null => params.push(Box::new(Option::<String>::None)),
            }
        }
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();
        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to execute search: {}", e)))?;

        // Parse rows, capturing the sort key for cursor construction.
        let mut parsed: Vec<(StoredResource, Option<CursorValue>)> = Vec::new();
        for row in &rows {
            let id: String = row.get(0);
            let version_id: String = row.get(1);
            let json_data: serde_json::Value = row.get(2);
            let last_updated: chrono::DateTime<Utc> = row.get(3);
            let fhir_version_str: String = row.get(4);
            let sort_key = keyset
                .as_ref()
                .map(|k| Self::read_cursor_value(row, 5, k.kind));

            let fhir_version = FhirVersion::from_storage(&fhir_version_str).unwrap_or_default();
            let resource = StoredResource::from_storage(
                resource_type.clone(),
                id,
                version_id,
                tenant.tenant_id().clone(),
                json_data,
                last_updated,
                last_updated,
                None,
                fhir_version,
            );
            parsed.push((resource, sort_key));
        }

        // Backward pagination fetched in reverse order — restore sort order.
        if cursor
            .as_ref()
            .map(|c| c.direction() == CursorDirection::Previous)
            .unwrap_or(false)
        {
            parsed.reverse();
        }

        // We fetched one extra to detect a further page.
        let has_next = parsed.len() > count;
        if has_next {
            parsed.pop();
        }

        let next_cursor = if has_next {
            parsed.last().map(|(r, sk)| {
                PageCursor::new(vec![sk.clone().unwrap_or(CursorValue::Null)], r.id()).encode()
            })
        } else {
            None
        };
        let previous_cursor = if has_previous {
            parsed.first().map(|(r, sk)| {
                PageCursor::previous(vec![sk.clone().unwrap_or(CursorValue::Null)], r.id()).encode()
            })
        } else {
            None
        };

        let resources: Vec<StoredResource> = parsed.into_iter().map(|(r, _)| r).collect();
        let page_info = PageInfo {
            next_cursor,
            previous_cursor,
            total: None,
            has_next,
            has_previous,
        };
        let page = Page::new(resources, page_info);

        Ok(SearchResult {
            resources: page,
            included: Vec::new(),
            total: None,
        })
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = &query.resource_type;

        let (sql, params): (
            String,
            Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
        ) = if !query.parameters.is_empty() {
            let filter = PostgresQueryBuilder::build_search_query(query, 2);

            let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
                Box::new(tenant_id.to_string()),
                Box::new(resource_type.to_string()),
            ];

            if let Some(ref fragment) = filter {
                for param in &fragment.params {
                    match param {
                        SqlParam::Text(s) => params.push(Box::new(s.clone())),
                        SqlParam::Float(f) => params.push(Box::new(*f)),
                        SqlParam::Integer(i) => params.push(Box::new(*i)),
                        SqlParam::Bool(b) => params.push(Box::new(*b)),
                        SqlParam::Timestamp(dt) => params.push(Box::new(*dt)),
                        SqlParam::Null => params.push(Box::new(Option::<String>::None)),
                    }
                }

                let sql = format!(
                    "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE AND ({})",
                    fragment.sql
                );
                (sql, params)
            } else {
                let sql = "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE".to_string();
                (sql, params)
            }
        } else {
            let sql = "SELECT COUNT(*) FROM resources WHERE tenant_id = $1 AND resource_type = $2 AND is_deleted = FALSE".to_string();
            let params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
                Box::new(tenant_id.to_string()),
                Box::new(resource_type.to_string()),
            ];
            (sql, params)
        };

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let row = client
            .query_one(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to count resources: {}", e)))?;

        let count: i64 = row.get(0);
        Ok(count as u64)
    }

    fn search_param_registry(
        &self,
    ) -> &std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        self.search_registry()
    }
}

#[async_trait]
impl MultiTypeSearchProvider for PostgresBackend {
    async fn search_multi(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let count = query.count.unwrap_or(100) as usize;
        let offset = query.offset.unwrap_or(0) as usize;

        // Build the type filter
        let type_filter = if resource_types.is_empty() {
            String::new()
        } else {
            let types: Vec<String> = resource_types
                .iter()
                .map(|t| format!("'{}'", t.replace('\'', "''")))
                .collect();
            format!(" AND resource_type IN ({})", types.join(", "))
        };

        let sql = format!(
            "SELECT resource_type, id, version_id, data, last_updated, fhir_version FROM resources
             WHERE tenant_id = $1 AND is_deleted = FALSE{}
             ORDER BY last_updated DESC, id DESC
             LIMIT {} OFFSET {}",
            type_filter,
            count + 1,
            offset
        );

        let rows = client
            .query(&sql, &[&tenant_id])
            .await
            .map_err(|e| internal_error(format!("Failed to execute multi-type search: {}", e)))?;

        let mut resources = Vec::new();
        for row in &rows {
            let res_type: String = row.get(0);
            let id: String = row.get(1);
            let version_id: String = row.get(2);
            let json_data: serde_json::Value = row.get(3);
            let last_updated: chrono::DateTime<Utc> = row.get(4);
            let fhir_version_str: String = row.get(5);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str).unwrap_or_default();

            let resource = StoredResource::from_storage(
                res_type,
                id,
                version_id,
                tenant.tenant_id().clone(),
                json_data,
                last_updated,
                last_updated,
                None,
                fhir_version,
            );

            resources.push(resource);
        }

        let has_next = resources.len() > count;
        if has_next {
            resources.pop();
        }

        let page_info = PageInfo {
            next_cursor: None,
            previous_cursor: None,
            total: None,
            has_next,
            has_previous: offset > 0,
        };

        Ok(SearchResult {
            resources: Page::new(resources, page_info),
            included: Vec::new(),
            total: None,
        })
    }
}

#[async_trait]
impl IncludeProvider for PostgresBackend {
    async fn resolve_includes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        if resources.is_empty() || includes.is_empty() {
            return Ok(Vec::new());
        }

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut included = Vec::new();
        let mut seen_refs: HashSet<String> = HashSet::new();

        for include in includes {
            for resource in resources {
                if resource.resource_type() != include.source_type {
                    continue;
                }

                let refs = Self::extract_references(resource.content(), &include.search_param);

                for reference in refs {
                    if let Some((ref_type, ref_id)) = Self::parse_reference(&reference) {
                        if let Some(ref target) = include.target_type {
                            if ref_type != *target {
                                continue;
                            }
                        }

                        let ref_key = format!("{}/{}", ref_type, ref_id);
                        if seen_refs.contains(&ref_key) {
                            continue;
                        }
                        seen_refs.insert(ref_key);

                        if let Some(included_resource) =
                            Self::fetch_resource(&client, tenant_id, &ref_type, &ref_id).await?
                        {
                            included.push(included_resource);
                        }
                    }
                }
            }
        }

        Ok(included)
    }
}

#[async_trait]
impl RevincludeProvider for PostgresBackend {
    async fn resolve_revincludes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        revincludes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        if resources.is_empty() || revincludes.is_empty() {
            return Ok(Vec::new());
        }

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut included = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        for revinclude in revincludes {
            let mut reference_values: Vec<String> = Vec::new();
            for resource in resources {
                reference_values.push(format!("{}/{}", resource.resource_type(), resource.id()));
                reference_values.push(resource.id().to_string());
            }

            if reference_values.is_empty() {
                continue;
            }

            // Use the search index to find resources referencing our results
            let placeholders: Vec<String> = (0..reference_values.len())
                .map(|i| format!("${}", i + 3))
                .collect();

            let sql = format!(
                "SELECT DISTINCT r.id, r.version_id, r.data, r.last_updated, r.fhir_version
                 FROM resources r
                 INNER JOIN search_index si ON r.tenant_id = si.tenant_id
                    AND r.resource_type = si.resource_type
                    AND r.id = si.resource_id
                 WHERE r.tenant_id = $1 AND r.resource_type = $2 AND r.is_deleted = FALSE
                 AND si.param_name = '{}'
                 AND si.value_reference IN ({})",
                revinclude.search_param,
                placeholders.join(", ")
            );

            let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
                Box::new(tenant_id.to_string()),
                Box::new(revinclude.source_type.clone()),
            ];
            for rv in &reference_values {
                params.push(Box::new(rv.clone()));
            }

            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                .iter()
                .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                .collect();

            let rows = client.query(&sql, &param_refs).await.map_err(|e| {
                internal_error(format!("Failed to execute revinclude query: {}", e))
            })?;

            for row in &rows {
                let id: String = row.get(0);
                let version_id: String = row.get(1);
                let json_data: serde_json::Value = row.get(2);
                let last_updated: chrono::DateTime<Utc> = row.get(3);
                let fhir_version_str: String = row.get(4);

                let resource_key = format!("{}/{}", revinclude.source_type, id);
                if seen_ids.contains(&resource_key) {
                    continue;
                }
                seen_ids.insert(resource_key);

                let fhir_version = FhirVersion::from_storage(&fhir_version_str).unwrap_or_default();

                let resource = StoredResource::from_storage(
                    &revinclude.source_type,
                    id,
                    version_id,
                    tenant.tenant_id().clone(),
                    json_data,
                    last_updated,
                    last_updated,
                    None,
                    fhir_version,
                );

                included.push(resource);
            }
        }

        Ok(included)
    }
}

#[async_trait]
impl ChainedSearchProvider for PostgresBackend {
    async fn resolve_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        chain: &str,
        value: &str,
    ) -> StorageResult<Vec<String>> {
        if chain.is_empty() {
            return Ok(Vec::new());
        }

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Build a multi-step chain query via the registry-driven builder.
        // The builder produces a `r.id IN (... nested SELECTs ...)` fragment
        // that handles arbitrary chain depth (was previously stubbed for >2
        // segments).
        let builder = ChainQueryBuilder::new(tenant_id, base_type, self.search_registry().clone())
            .with_param_offset(1);
        let parsed = builder
            .parse_chain(chain)
            .map_err(|e| internal_error(format!("Failed to parse chain: {}", e)))?;
        let parsed_value = crate::types::SearchValue::parse(value);
        let fragment = builder.build_forward_chain_sql(&parsed, &parsed_value)?;

        let sql = format!(
            "SELECT r.id FROM resources r WHERE r.tenant_id = $1 \
             AND r.resource_type = '{base}' AND r.is_deleted = FALSE AND {clause}",
            base = base_type,
            clause = fragment.sql,
        );

        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            vec![Box::new(tenant_id.to_string())];
        for p in &fragment.params {
            match p {
                SqlParam::Text(s) => params.push(Box::new(s.clone())),
                SqlParam::Float(f) => params.push(Box::new(*f)),
                SqlParam::Integer(i) => params.push(Box::new(*i)),
                SqlParam::Bool(b) => params.push(Box::new(*b)),
                SqlParam::Timestamp(dt) => params.push(Box::new(*dt)),
                SqlParam::Null => params.push(Box::new(Option::<String>::None)),
            }
        }
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to execute chain query: {}", e)))?;

        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    async fn resolve_reverse_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        reverse_chain: &ReverseChainedParameter,
    ) -> StorageResult<Vec<String>> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();

        // Use the registry-driven builder so we handle nested `_has` chains
        // and any param type (was previously single-level only with hardcoded
        // token-or-string-or-empty fallback).
        let builder = ChainQueryBuilder::new(tenant_id, base_type, self.search_registry().clone())
            .with_param_offset(1);
        let fragment = builder.build_reverse_chain_sql(reverse_chain)?;

        let sql = format!(
            "SELECT r.id FROM resources r WHERE r.tenant_id = $1 \
             AND r.resource_type = '{base}' AND r.is_deleted = FALSE AND {clause}",
            base = base_type,
            clause = fragment.sql,
        );

        let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> =
            vec![Box::new(tenant_id.to_string())];
        for p in &fragment.params {
            match p {
                SqlParam::Text(s) => params.push(Box::new(s.clone())),
                SqlParam::Float(f) => params.push(Box::new(*f)),
                SqlParam::Integer(i) => params.push(Box::new(*i)),
                SqlParam::Bool(b) => params.push(Box::new(*b)),
                SqlParam::Timestamp(dt) => params.push(Box::new(*dt)),
                SqlParam::Null => params.push(Box::new(Option::<String>::None)),
            }
        }
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let rows = client
            .query(&sql, &param_refs)
            .await
            .map_err(|e| internal_error(format!("Failed to execute reverse chain query: {}", e)))?;

        Ok(rows.iter().map(|r| r.get(0)).collect())
    }
}

#[async_trait]
impl TextSearchProvider for PostgresBackend {
    async fn search_text(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        text: &str,
        pagination: &Pagination,
    ) -> StorageResult<SearchResult> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let count = pagination.count as usize;

        // Use PostgreSQL native FTS with tsvector/tsquery
        let sql = format!(
            "SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
                    ts_rank(fts.narrative_tsvector, plainto_tsquery('english', $3)) AS rank
             FROM resources r
             INNER JOIN resource_fts fts ON r.tenant_id = fts.tenant_id
                AND r.resource_type = fts.resource_type AND r.id = fts.resource_id
             WHERE r.tenant_id = $1 AND r.resource_type = $2 AND r.is_deleted = FALSE
             AND fts.narrative_tsvector @@ plainto_tsquery('english', $3)
             ORDER BY rank DESC, r.last_updated DESC
             LIMIT {}",
            count + 1
        );

        let rows = client
            .query(&sql, &[&tenant_id, &resource_type, &text])
            .await
            .map_err(|e| internal_error(format!("Failed to execute text search: {}", e)))?;

        let mut resources = Vec::new();
        for row in &rows {
            let id: String = row.get(0);
            let version_id: String = row.get(1);
            let json_data: serde_json::Value = row.get(2);
            let last_updated: chrono::DateTime<Utc> = row.get(3);
            let fhir_version_str: String = row.get(4);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str).unwrap_or_default();

            resources.push(StoredResource::from_storage(
                resource_type,
                id,
                version_id,
                tenant.tenant_id().clone(),
                json_data,
                last_updated,
                last_updated,
                None,
                fhir_version,
            ));
        }

        let has_next = resources.len() > count;
        if has_next {
            resources.pop();
        }

        let page_info = PageInfo {
            next_cursor: None,
            previous_cursor: None,
            total: None,
            has_next,
            has_previous: false,
        };

        Ok(SearchResult {
            resources: Page::new(resources, page_info),
            included: Vec::new(),
            total: None,
        })
    }

    async fn search_content(
        &self,
        tenant: &TenantContext,
        resource_type: &str,
        content: &str,
        pagination: &Pagination,
    ) -> StorageResult<SearchResult> {
        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let count = pagination.count as usize;

        // Use content_tsvector for _content search
        let sql = format!(
            "SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
                    ts_rank(fts.content_tsvector, plainto_tsquery('english', $3)) AS rank
             FROM resources r
             INNER JOIN resource_fts fts ON r.tenant_id = fts.tenant_id
                AND r.resource_type = fts.resource_type AND r.id = fts.resource_id
             WHERE r.tenant_id = $1 AND r.resource_type = $2 AND r.is_deleted = FALSE
             AND fts.content_tsvector @@ plainto_tsquery('english', $3)
             ORDER BY rank DESC, r.last_updated DESC
             LIMIT {}",
            count + 1
        );

        let rows = client
            .query(&sql, &[&tenant_id, &resource_type, &content])
            .await
            .map_err(|e| internal_error(format!("Failed to execute content search: {}", e)))?;

        let mut resources = Vec::new();
        for row in &rows {
            let id: String = row.get(0);
            let version_id: String = row.get(1);
            let json_data: serde_json::Value = row.get(2);
            let last_updated: chrono::DateTime<Utc> = row.get(3);
            let fhir_version_str: String = row.get(4);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str).unwrap_or_default();

            resources.push(StoredResource::from_storage(
                resource_type,
                id,
                version_id,
                tenant.tenant_id().clone(),
                json_data,
                last_updated,
                last_updated,
                None,
                fhir_version,
            ));
        }

        let has_next = resources.len() > count;
        if has_next {
            resources.pop();
        }

        let page_info = PageInfo {
            next_cursor: None,
            previous_cursor: None,
            total: None,
            has_next,
            has_previous: false,
        };

        Ok(SearchResult {
            resources: Page::new(resources, page_info),
            included: Vec::new(),
            total: None,
        })
    }
}

// Helper methods for search implementations
impl PostgresBackend {
    /// Extract timestamp and ID from a cursor for keyset pagination.
    /// Binds the cursor's boundary sort value as `$3`, typed per the sort key
    /// kind so PostgreSQL compares it correctly against the sort expression.
    fn bind_cursor_value(
        params: &mut Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
        kind: SortValueKind,
        cursor: &PageCursor,
    ) -> StorageResult<()> {
        let value = cursor.sort_values().first();
        match kind {
            SortValueKind::Timestamp => {
                let dt = match value {
                    Some(CursorValue::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
                        .map(|d| d.with_timezone(&Utc))
                        .map_err(|_| internal_error("Invalid cursor timestamp".to_string()))?,
                    _ => {
                        return Err(internal_error(
                            "Invalid cursor: expected timestamp".to_string(),
                        ));
                    }
                };
                params.push(Box::new(dt));
            }
            SortValueKind::Number => {
                let n = match value {
                    Some(CursorValue::Decimal(f)) => *f,
                    Some(CursorValue::Number(i)) => *i as f64,
                    Some(CursorValue::String(s)) => s.parse().unwrap_or(0.0),
                    _ => {
                        return Err(internal_error(
                            "Invalid cursor: expected number".to_string(),
                        ));
                    }
                };
                params.push(Box::new(n));
            }
            SortValueKind::Text => match value {
                Some(CursorValue::String(s)) => params.push(Box::new(s.clone())),
                Some(CursorValue::Null) | None => params.push(Box::new(Option::<String>::None)),
                _ => {
                    return Err(internal_error("Invalid cursor: expected text".to_string()));
                }
            },
        }
        Ok(())
    }

    /// Reads the `sort_key` column (index 5) as a `CursorValue` per the key kind.
    fn read_cursor_value(
        row: &tokio_postgres::Row,
        idx: usize,
        kind: SortValueKind,
    ) -> CursorValue {
        match kind {
            SortValueKind::Timestamp => {
                let v: Option<chrono::DateTime<Utc>> = row.try_get(idx).ok().flatten();
                v.map(|d| CursorValue::String(d.to_rfc3339()))
                    .unwrap_or(CursorValue::Null)
            }
            SortValueKind::Number => {
                let v: Option<f64> = row.try_get(idx).ok().flatten();
                v.map(CursorValue::Decimal).unwrap_or(CursorValue::Null)
            }
            SortValueKind::Text => {
                let v: Option<String> = row.try_get(idx).ok().flatten();
                v.map(CursorValue::String).unwrap_or(CursorValue::Null)
            }
        }
    }

    /// Extract references from a resource for a given search parameter.
    fn extract_references(content: &serde_json::Value, search_param: &str) -> Vec<String> {
        let mut refs = Vec::new();
        if let Some(value) = content.get(search_param) {
            Self::collect_references_from_value(value, &mut refs);
        }
        refs
    }

    /// Recursively collect reference strings from a JSON value.
    fn collect_references_from_value(value: &serde_json::Value, refs: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(obj) => {
                if let Some(serde_json::Value::String(ref_str)) = obj.get("reference") {
                    refs.push(ref_str.clone());
                }
                for v in obj.values() {
                    Self::collect_references_from_value(v, refs);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    Self::collect_references_from_value(item, refs);
                }
            }
            _ => {}
        }
    }

    /// Parse a reference string into (type, id).
    fn parse_reference(reference: &str) -> Option<(String, String)> {
        let path = reference
            .strip_prefix("http://")
            .or_else(|| reference.strip_prefix("https://"))
            .map(|s| s.rsplit('/').take(2).collect::<Vec<_>>())
            .unwrap_or_else(|| reference.split('/').collect());

        if path.len() >= 2 {
            if reference.starts_with("http") {
                Some((path[1].to_string(), path[0].to_string()))
            } else {
                Some((path[0].to_string(), path[1].to_string()))
            }
        } else {
            None
        }
    }

    /// Fetch a single resource by type and ID.
    async fn fetch_resource(
        client: &deadpool_postgres::Client,
        tenant_id: &str,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let rows = client
            .query(
                "SELECT version_id, data, last_updated, fhir_version FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
                &[&tenant_id, &resource_type, &id],
            )
            .await
            .map_err(|e| internal_error(format!("Failed to fetch resource: {}", e)))?;

        if rows.is_empty() {
            return Ok(None);
        }

        let row = &rows[0];
        let version_id: String = row.get(0);
        let json_data: serde_json::Value = row.get(1);
        let last_updated: chrono::DateTime<Utc> = row.get(2);
        let fhir_version_str: String = row.get(3);
        let fhir_version = FhirVersion::from_storage(&fhir_version_str).unwrap_or_default();

        Ok(Some(StoredResource::from_storage(
            resource_type,
            id,
            version_id,
            crate::tenant::TenantId::new(tenant_id),
            json_data,
            last_updated,
            last_updated,
            None,
            fhir_version,
        )))
    }
}
