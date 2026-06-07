//! Search implementation for SQLite backend.
//!
//! This module provides search functionality for the SQLite backend including:
//! - Basic single-type search
//! - Multi-type search
//! - _include and _revinclude support
//! - Chained search parameter support
//! - Search parameter filtering using the search_index table

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;
use rusqlite::params;

use crate::core::{
    ChainedSearchProvider, IncludeProvider, MultiTypeSearchProvider, RevincludeProvider,
    SearchProvider, SearchResult,
};
use crate::error::{BackendError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    CursorDirection, CursorValue, IncludeDirective, Page, PageCursor, PageInfo,
    ReverseChainedParameter, SearchQuery, SearchValue, StoredResource,
};

use super::SqliteBackend;
use super::search::{QueryBuilder, SortValueKind, SqlParam};

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
}

/// Binds the cursor's boundary sort value as `?3`, typed per the sort key kind.
/// Timestamps are stored as RFC3339 text, so they bind (and compare) as text.
fn bind_cursor_value(
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    kind: SortValueKind,
    cursor: &PageCursor,
) -> StorageResult<()> {
    let value = cursor.sort_values().first();
    match kind {
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
        SortValueKind::Timestamp | SortValueKind::Text => match value {
            Some(CursorValue::String(s)) => params.push(Box::new(s.clone())),
            Some(CursorValue::Null) | None => params.push(Box::new(Option::<String>::None)),
            _ => {
                return Err(internal_error("Invalid cursor: expected text".to_string()));
            }
        },
    }
    Ok(())
}

#[async_trait]
impl SearchProvider for SqliteBackend {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        // Populate Bundle.total only when the client asked for it
        // (`_total=accurate|estimate`). Computed up-front, before acquiring the
        // (non-Send) connection, so it is not held across this await.
        let total = if query.wants_total() {
            Some(self.search_count(tenant, query).await?)
        } else {
            None
        };

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = &query.resource_type;

        // Get count with default
        let count = query.count.unwrap_or(100) as usize;

        // Keyset key for cursor pagination. `None` for multi-field sorts, which
        // are returned as a single page rather than paged with an inconsistent
        // keyset.
        let keyset = QueryBuilder::new(tenant_id, resource_type).primary_keyset_key(query);

        // Only honor an inbound cursor when we can build a keyset comparison.
        let cursor = if keyset.is_some() {
            query
                .cursor
                .as_ref()
                .and_then(|c| PageCursor::decode(c).ok())
        } else {
            None
        };

        // Param layout: ?1=tenant, ?2=type, then (cursor) ?3=sort value, ?4=id,
        // then the search-filter params.
        let param_offset = if cursor.is_some() { 4 } else { 2 };

        let search_filter = if !query.parameters.is_empty() {
            let builder =
                QueryBuilder::new(tenant_id, resource_type).with_param_offset(param_offset);
            let fragment = builder.build(query);
            if !fragment.sql.is_empty() {
                Some(fragment)
            } else {
                None
            }
        } else {
            None
        };
        let filter_clause = search_filter
            .as_ref()
            .map(|f| format!(" AND id IN ({})", f.sql))
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
            QueryBuilder::new(tenant_id, resource_type).build_order_by(query)
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
                         WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0{filter} \
                         AND ({e} {e_op} ?3 OR ({e} = ?3 AND id > ?4)) \
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
                         WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0{filter} \
                         AND ({e} {e_op} ?3 OR ({e} = ?3 AND id < ?4)) \
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
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0{filter} \
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
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0{filter} \
                 {order} LIMIT {lim}",
                cols = select_cols,
                filter = filter_clause,
                order = order_by,
                lim = count + 1,
            );
            (sql, false)
        };

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare search query: {}", e)))?;

        // Bind params: tenant, type, then (cursor) sort value + id, then filter params.
        let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(tenant_id.to_string()),
            Box::new(resource_type.to_string()),
        ];
        if let (Some(cursor), Some(k)) = (&cursor, &keyset) {
            bind_cursor_value(&mut all_params, k.kind, cursor)?;
            all_params.push(Box::new(cursor.resource_id().to_string()));
        }
        for param in &search_params {
            match param {
                SqlParam::String(s) => all_params.push(Box::new(s.clone())),
                SqlParam::Integer(i) => all_params.push(Box::new(*i)),
                SqlParam::Float(f) => all_params.push(Box::new(*f)),
                SqlParam::Null => all_params.push(Box::new(Option::<String>::None)),
            }
        }
        let param_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

        // The sort key (column 5) is selected only when keyset paging is active.
        let sort_kind = keyset.as_ref().map(|k| k.kind);
        let raw_rows: Vec<(String, String, Vec<u8>, String, String, Option<CursorValue>)> = stmt
            .query_map(param_refs.as_slice(), |row| {
                let id: String = row.get(0)?;
                let version_id: String = row.get(1)?;
                let data: Vec<u8> = row.get(2)?;
                let last_updated: String = row.get(3)?;
                let fhir_version: String = row.get(4)?;
                let sort_key = match sort_kind {
                    Some(SortValueKind::Number) => {
                        row.get::<_, Option<f64>>(5)?.map(CursorValue::Decimal)
                    }
                    // Timestamps are stored as RFC3339 text, so read text.
                    Some(_) => row.get::<_, Option<String>>(5)?.map(CursorValue::String),
                    None => None,
                };
                Ok((id, version_id, data, last_updated, fhir_version, sort_key))
            })
            .map_err(|e| internal_error(format!("Failed to execute search: {}", e)))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| internal_error(format!("Failed to read row: {}", e)))?;

        // Parse rows, carrying the sort key for cursor construction.
        let mut parsed: Vec<(StoredResource, Option<CursorValue>)> = Vec::new();
        for (id, version_id, data, last_updated_str, fhir_version_str, sort_key) in raw_rows {
            let json_data: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|e| internal_error(format!("Failed to deserialize resource: {}", e)))?;

            let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                .with_timezone(&Utc);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

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
            total,
            has_next,
            has_previous,
        };

        let page = Page::new(resources, page_info);

        Ok(SearchResult {
            resources: page,
            included: Vec::new(),
            total,
        })
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = &query.resource_type;

        // Build the search filter if there are search parameters
        let (sql, all_params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if !query
            .parameters
            .is_empty()
        {
            let builder = QueryBuilder::new(tenant_id, resource_type).with_param_offset(2);
            let fragment = builder.build(query);

            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![
                Box::new(tenant_id.to_string()),
                Box::new(resource_type.to_string()),
            ];

            // Add search params
            for param in &fragment.params {
                match param {
                    SqlParam::String(s) => params.push(Box::new(s.clone())),
                    SqlParam::Integer(i) => params.push(Box::new(*i)),
                    SqlParam::Float(f) => params.push(Box::new(*f)),
                    SqlParam::Null => params.push(Box::new(Option::<String>::None)),
                }
            }

            let sql = format!(
                "SELECT COUNT(*) FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 AND id IN ({})",
                fragment.sql
            );

            (sql, params)
        } else {
            let sql = "SELECT COUNT(*) FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0".to_string();
            let params: Vec<Box<dyn rusqlite::ToSql>> = vec![
                Box::new(tenant_id.to_string()),
                Box::new(resource_type.to_string()),
            ];
            (sql, params)
        };

        let param_refs: Vec<&dyn rusqlite::ToSql> = all_params.iter().map(|p| p.as_ref()).collect();

        let count: i64 = conn
            .query_row(&sql, param_refs.as_slice(), |row| row.get(0))
            .map_err(|e| internal_error(format!("Failed to count resources: {}", e)))?;

        Ok(count as u64)
    }

    fn search_param_registry(
        &self,
    ) -> &std::sync::Arc<parking_lot::RwLock<crate::search::SearchParameterRegistry>> {
        self.search_registry()
    }
}

#[async_trait]
impl MultiTypeSearchProvider for SqliteBackend {
    async fn search_multi(
        &self,
        tenant: &TenantContext,
        resource_types: &[&str],
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Get count and offset with defaults
        let count = query.count.unwrap_or(100) as usize;
        let offset = query.offset.unwrap_or(0) as usize;

        // Build the type filter
        let type_filter = if resource_types.is_empty() {
            // No filter - search all types
            String::new()
        } else {
            // Filter to specific types
            let types: Vec<String> = resource_types
                .iter()
                .map(|t| format!("'{}'", t.replace('\'', "''")))
                .collect();
            format!(" AND resource_type IN ({})", types.join(", "))
        };

        let sql = format!(
            "SELECT resource_type, id, version_id, data, last_updated, fhir_version FROM resources
             WHERE tenant_id = ?1 AND is_deleted = 0{}
             ORDER BY last_updated DESC
             LIMIT {} OFFSET {}",
            type_filter,
            count + 1,
            offset
        );

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare multi-type search: {}", e)))?;

        let rows = stmt
            .query_map(params![tenant_id], |row| {
                let resource_type: String = row.get(0)?;
                let id: String = row.get(1)?;
                let version_id: String = row.get(2)?;
                let data: Vec<u8> = row.get(3)?;
                let last_updated: String = row.get(4)?;
                let fhir_version: String = row.get(5)?;
                Ok((
                    resource_type,
                    id,
                    version_id,
                    data,
                    last_updated,
                    fhir_version,
                ))
            })
            .map_err(|e| internal_error(format!("Failed to execute multi-type search: {}", e)))?;

        let mut resources = Vec::new();
        for row in rows {
            let (resource_type, id, version_id, data, last_updated_str, fhir_version_str) =
                row.map_err(|e| internal_error(format!("Failed to read row: {}", e)))?;

            let json_data: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|e| internal_error(format!("Failed to deserialize resource: {}", e)))?;

            let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                .with_timezone(&Utc);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            let resource = StoredResource::from_storage(
                resource_type,
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

        // Check if there are more results
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
impl IncludeProvider for SqliteBackend {
    async fn resolve_includes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        includes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        if resources.is_empty() || includes.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut included = Vec::new();
        let mut seen_refs: HashSet<String> = HashSet::new();

        for include in includes {
            // For each resource, extract references for the include parameter
            for resource in resources {
                // Skip if source type doesn't match
                if resource.resource_type() != include.source_type {
                    continue;
                }

                // Extract references from the resource based on the search parameter
                let refs = self.extract_references(resource.content(), &include.search_param);

                for reference in refs {
                    // Parse the reference (e.g., "Patient/123")
                    if let Some((ref_type, ref_id)) = self.parse_reference(&reference) {
                        // Apply target type filter if specified
                        if let Some(ref target) = include.target_type {
                            if ref_type != *target {
                                continue;
                            }
                        }

                        // Skip if we've already included this resource
                        let ref_key = format!("{}/{}", ref_type, ref_id);
                        if seen_refs.contains(&ref_key) {
                            continue;
                        }
                        seen_refs.insert(ref_key);

                        // Fetch the referenced resource
                        if let Some(included_resource) =
                            self.fetch_resource(&conn, tenant_id, &ref_type, &ref_id)?
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
impl RevincludeProvider for SqliteBackend {
    async fn resolve_revincludes(
        &self,
        tenant: &TenantContext,
        resources: &[StoredResource],
        revincludes: &[IncludeDirective],
    ) -> StorageResult<Vec<StoredResource>> {
        if resources.is_empty() || revincludes.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        let mut included = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        for revinclude in revincludes {
            // Build the list of references to search for
            let mut reference_values: Vec<String> = Vec::new();
            for resource in resources {
                // For _revinclude, we look for resources that reference our results
                // The reference format is typically "ResourceType/id"
                reference_values.push(format!("{}/{}", resource.resource_type(), resource.id()));
                // Also check just the ID in case the reference doesn't include the type
                reference_values.push(resource.id().to_string());
            }

            if reference_values.is_empty() {
                continue;
            }

            // Search for resources of source_type that reference our resources
            let reference_pattern = reference_values
                .iter()
                .map(|r| format!("%{}%", r.replace('%', "\\%").replace('_', "\\_")))
                .collect::<Vec<_>>();

            // Build SQL to find resources containing any of the references
            // We search in the JSON data for the search_param field containing a reference
            let sql = format!(
                "SELECT id, version_id, data, last_updated, fhir_version FROM resources
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0
                 AND ({})",
                reference_pattern
                    .iter()
                    .map(|_| "data LIKE ?".to_string())
                    .collect::<Vec<_>>()
                    .join(" OR ")
            );

            let mut stmt = conn.prepare(&sql).map_err(|e| {
                internal_error(format!("Failed to prepare revinclude query: {}", e))
            })?;

            // Build params: tenant_id, source_type, then all the patterns
            let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            param_values.push(Box::new(tenant_id.to_string()));
            param_values.push(Box::new(revinclude.source_type.clone()));
            for pattern in &reference_pattern {
                param_values.push(Box::new(pattern.clone()));
            }

            let param_refs: Vec<&dyn rusqlite::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    let id: String = row.get(0)?;
                    let version_id: String = row.get(1)?;
                    let data: Vec<u8> = row.get(2)?;
                    let last_updated: String = row.get(3)?;
                    let fhir_version: String = row.get(4)?;
                    Ok((id, version_id, data, last_updated, fhir_version))
                })
                .map_err(|e| {
                    internal_error(format!("Failed to execute revinclude query: {}", e))
                })?;

            for row in rows {
                let (id, version_id, data, last_updated_str, fhir_version_str) =
                    row.map_err(|e| internal_error(format!("Failed to read row: {}", e)))?;

                // Skip if we've already included this resource
                let resource_key = format!("{}/{}", revinclude.source_type, id);
                if seen_ids.contains(&resource_key) {
                    continue;
                }

                let json_data: serde_json::Value = serde_json::from_slice(&data)
                    .map_err(|e| internal_error(format!("Failed to deserialize: {}", e)))?;

                // Verify this resource actually references one of our results via the search_param
                if !self.verify_reference(&json_data, &revinclude.search_param, &reference_values) {
                    continue;
                }

                seen_ids.insert(resource_key);

                let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                    .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                    .with_timezone(&Utc);

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

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
impl ChainedSearchProvider for SqliteBackend {
    async fn resolve_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        chain: &str,
        value: &str,
    ) -> StorageResult<Vec<String>> {
        use super::search::ChainQueryBuilder;

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        if chain.is_empty() {
            return Ok(Vec::new());
        }

        // Create the chain query builder with registry access
        let builder = ChainQueryBuilder::new(tenant_id, base_type, self.get_search_registry())
            .with_param_offset(2); // After ?1 (tenant) and ?2 (resource_type)

        // Parse the chain
        let parsed = match builder.parse_chain(chain) {
            Ok(p) => p,
            Err(e) => {
                return Err(internal_error(format!("Failed to parse chain: {}", e)));
            }
        };

        // Build the SQL fragment
        let search_value = SearchValue::eq(value);
        let fragment = match builder.build_forward_chain_sql(&parsed, &search_value) {
            Ok(f) => f,
            Err(e) => {
                return Err(internal_error(format!("Failed to build chain SQL: {}", e)));
            }
        };

        // Execute the query to get matching IDs
        // The fragment generates: r.id IN (SELECT ...)
        // We need to wrap it in a proper SELECT FROM resources
        let sql = format!(
            "SELECT DISTINCT r.id FROM resources r \
             WHERE r.tenant_id = ?1 AND r.resource_type = ?2 AND r.is_deleted = 0 AND {}",
            fragment.sql
        );

        // Bind parameters: tenant_id, resource_type, then fragment params
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare chain query: {}", e)))?;

        // Build parameter vector for rusqlite
        let mut bound_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        bound_params.push(Box::new(tenant_id.to_string()));
        bound_params.push(Box::new(base_type.to_string()));
        for param in &fragment.params {
            match param {
                SqlParam::String(s) => bound_params.push(Box::new(s.clone())),
                SqlParam::Integer(i) => bound_params.push(Box::new(*i)),
                SqlParam::Float(f) => bound_params.push(Box::new(*f)),
                SqlParam::Null => bound_params.push(Box::new(rusqlite::types::Null)),
            }
        }

        let params_ref: Vec<&dyn rusqlite::ToSql> =
            bound_params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), |row| row.get::<_, String>(0))
            .map_err(|e| internal_error(format!("Failed to execute chain query: {}", e)))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| internal_error(format!("Failed to read row: {}", e)))?);
        }

        Ok(ids)
    }

    async fn resolve_reverse_chain(
        &self,
        tenant: &TenantContext,
        base_type: &str,
        reverse_chain: &ReverseChainedParameter,
    ) -> StorageResult<Vec<String>> {
        use super::search::ChainQueryBuilder;

        let conn = self.get_connection()?;
        let tenant_id = tenant.tenant_id().as_str();

        // Create the chain query builder with registry access
        let builder = ChainQueryBuilder::new(tenant_id, base_type, self.get_search_registry())
            .with_param_offset(2); // After ?1 (tenant) and ?2 (resource_type)

        // Build the SQL fragment for reverse chain
        let fragment = match builder.build_reverse_chain_sql(reverse_chain) {
            Ok(f) => f,
            Err(e) => {
                return Err(internal_error(format!(
                    "Failed to build reverse chain SQL: {}",
                    e
                )));
            }
        };

        // Execute the query to get matching IDs
        // The fragment generates: r.id IN (SELECT ...)
        let sql = format!(
            "SELECT DISTINCT r.id FROM resources r \
             WHERE r.tenant_id = ?1 AND r.resource_type = ?2 AND r.is_deleted = 0 AND {}",
            fragment.sql
        );

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare reverse chain query: {}", e)))?;

        // Build parameter vector for rusqlite
        let mut bound_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        bound_params.push(Box::new(tenant_id.to_string()));
        bound_params.push(Box::new(base_type.to_string()));
        for param in &fragment.params {
            match param {
                SqlParam::String(s) => bound_params.push(Box::new(s.clone())),
                SqlParam::Integer(i) => bound_params.push(Box::new(*i)),
                SqlParam::Float(f) => bound_params.push(Box::new(*f)),
                SqlParam::Null => bound_params.push(Box::new(rusqlite::types::Null)),
            }
        }

        let params_ref: Vec<&dyn rusqlite::ToSql> =
            bound_params.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), |row| row.get::<_, String>(0))
            .map_err(|e| internal_error(format!("Failed to execute reverse chain query: {}", e)))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| internal_error(format!("Failed to read row: {}", e)))?);
        }

        Ok(ids)
    }
}

// Helper methods for search implementations
impl SqliteBackend {
    /// Extract references from a resource for a given search parameter.
    fn extract_references(&self, content: &serde_json::Value, search_param: &str) -> Vec<String> {
        let mut refs = Vec::new();

        // Try direct field access (e.g., "subject" -> content.subject)
        if let Some(value) = content.get(search_param) {
            self.collect_references_from_value(value, &mut refs);
        }

        // Try common reference field patterns
        // Many FHIR references are in fields like "patient", "subject", "performer", etc.
        // and contain a "reference" sub-field
        refs
    }

    /// Recursively collect reference strings from a JSON value.
    #[allow(clippy::only_used_in_recursion)]
    fn collect_references_from_value(&self, value: &serde_json::Value, refs: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(obj) => {
                // Check for "reference" field
                if let Some(serde_json::Value::String(ref_str)) = obj.get("reference") {
                    refs.push(ref_str.clone());
                }
                // Recurse into object fields
                for v in obj.values() {
                    self.collect_references_from_value(v, refs);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    self.collect_references_from_value(item, refs);
                }
            }
            _ => {}
        }
    }

    /// Parse a reference string into (type, id).
    fn parse_reference(&self, reference: &str) -> Option<(String, String)> {
        // Handle formats:
        // - "Patient/123"
        // - "http://example.com/fhir/Patient/123"
        let path = reference
            .strip_prefix("http://")
            .or_else(|| reference.strip_prefix("https://"))
            .map(|s| s.rsplit('/').take(2).collect::<Vec<_>>())
            .unwrap_or_else(|| reference.split('/').collect());

        if path.len() >= 2 {
            // For URL format, path is reversed
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
    fn fetch_resource(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        id: &str,
    ) -> StorageResult<Option<StoredResource>> {
        let result = conn.query_row(
            "SELECT version_id, data, last_updated, fhir_version FROM resources
             WHERE tenant_id = ?1 AND resource_type = ?2 AND id = ?3 AND is_deleted = 0",
            params![tenant_id, resource_type, id],
            |row| {
                let version_id: String = row.get(0)?;
                let data: Vec<u8> = row.get(1)?;
                let last_updated: String = row.get(2)?;
                let fhir_version: String = row.get(3)?;
                Ok((version_id, data, last_updated, fhir_version))
            },
        );

        match result {
            Ok((version_id, data, last_updated_str, fhir_version_str)) => {
                let json_data: serde_json::Value = serde_json::from_slice(&data)
                    .map_err(|e| internal_error(format!("Failed to deserialize: {}", e)))?;

                let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                    .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                    .with_timezone(&Utc);

                let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                    .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

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
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(internal_error(format!("Failed to fetch resource: {}", e))),
        }
    }

    /// Verify that a resource contains a reference to one of the given values.
    fn verify_reference(
        &self,
        content: &serde_json::Value,
        search_param: &str,
        reference_values: &[String],
    ) -> bool {
        let refs = self.extract_references(content, search_param);
        for ref_str in refs {
            // Check full reference
            if reference_values.iter().any(|v| ref_str.contains(v)) {
                return true;
            }
            // Check just the ID part
            if let Some((_, ref_id)) = self.parse_reference(&ref_str) {
                if reference_values.contains(&ref_id) {
                    return true;
                }
            }
        }
        false
    }

    /// Find resources matching a simple field value search using the search index.
    #[allow(dead_code)]
    fn find_resources_by_value(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
        param_name: &str,
        value: &str,
    ) -> StorageResult<Vec<String>> {
        // Use the pre-computed search_index table instead
        // This is consistent with our PrecomputedIndex strategy

        // Handle token format (system|code or just code)
        let (system_clause, search_value) = if value.contains('|') {
            let parts: Vec<&str> = value.splitn(2, '|').collect();
            if parts.len() == 2 && !parts[0].is_empty() {
                // system|code format
                (
                    format!(
                        "AND value_token_system = '{}'",
                        parts[0].replace('\'', "''")
                    ),
                    parts[1].to_string(),
                )
            } else if parts.len() == 2 {
                // |code format (no system)
                (
                    "AND (value_token_system IS NULL OR value_token_system = '')".to_string(),
                    parts[1].to_string(),
                )
            } else {
                (String::new(), value.to_string())
            }
        } else {
            (String::new(), value.to_string())
        };

        let escaped_value = search_value.replace('\'', "''");

        // Query the search_index table for matching resources
        // Search across string, token code, and reference values
        let sql = format!(
            "SELECT DISTINCT resource_id FROM search_index
             WHERE tenant_id = ?1 AND resource_type = ?2 AND param_name = ?3
             AND (
                 value_string LIKE '%{}%' COLLATE NOCASE
                 OR value_token_code = '{}'
                 OR value_token_code LIKE '%{}%'
                 OR value_reference LIKE '%{}%'
             )
             {}",
            escaped_value, escaped_value, escaped_value, escaped_value, system_clause
        );

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| internal_error(format!("Failed to prepare find query: {}", e)))?;

        let rows = stmt
            .query_map(params![tenant_id, resource_type, param_name], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| internal_error(format!("Failed to execute find query: {}", e)))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| internal_error(format!("Failed to read row: {}", e)))?);
        }

        Ok(ids)
    }

    /// Get all resources of a type for a tenant.
    #[allow(dead_code)]
    fn get_all_resources(
        &self,
        conn: &rusqlite::Connection,
        tenant_id: &str,
        resource_type: &str,
    ) -> StorageResult<Vec<StoredResource>> {
        let mut stmt = conn
            .prepare(
                "SELECT id, version_id, data, last_updated, fhir_version FROM resources
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0",
            )
            .map_err(|e| internal_error(format!("Failed to prepare query: {}", e)))?;

        let rows = stmt
            .query_map(params![tenant_id, resource_type], |row| {
                let id: String = row.get(0)?;
                let version_id: String = row.get(1)?;
                let data: Vec<u8> = row.get(2)?;
                let last_updated: String = row.get(3)?;
                let fhir_version: String = row.get(4)?;
                Ok((id, version_id, data, last_updated, fhir_version))
            })
            .map_err(|e| internal_error(format!("Failed to query resources: {}", e)))?;

        let mut resources = Vec::new();
        for row in rows {
            let (id, version_id, data, last_updated_str, fhir_version_str) =
                row.map_err(|e| internal_error(format!("Failed to read row: {}", e)))?;

            let json_data: serde_json::Value = serde_json::from_slice(&data)
                .map_err(|e| internal_error(format!("Failed to deserialize: {}", e)))?;

            let last_updated = chrono::DateTime::parse_from_rfc3339(&last_updated_str)
                .map_err(|e| internal_error(format!("Failed to parse last_updated: {}", e)))?
                .with_timezone(&Utc);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

            resources.push(StoredResource::from_storage(
                resource_type,
                id,
                version_id,
                crate::tenant::TenantId::new(tenant_id),
                json_data,
                last_updated,
                last_updated,
                None,
                fhir_version,
            ));
        }

        Ok(resources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ResourceStorage;
    use crate::tenant::{TenantId, TenantPermissions};
    use crate::types::SearchParameter;
    use serde_json::json;

    fn create_test_backend() -> SqliteBackend {
        // Point at the workspace's data directory so the search-parameter
        // registry loads the full FHIR spec (otherwise only the 5 minimal
        // embedded params are available and chained-search tests fail to
        // resolve param types like Observation.code → Token).
        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("data");
        let mut config = crate::backends::sqlite::backend::SqliteBackendConfig::default();
        config.data_dir = Some(data_dir);
        let backend = SqliteBackend::with_config(":memory:", config).unwrap();
        backend.init_schema().unwrap();
        backend
    }

    fn create_test_tenant() -> TenantContext {
        TenantContext::new(
            TenantId::new("test-tenant"),
            TenantPermissions::full_access(),
        )
    }

    #[tokio::test]
    async fn test_search_empty() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let query = SearchQuery::new("Patient");
        let result = backend.search(&tenant, &query).await.unwrap();

        assert!(result.resources.items.is_empty());
    }

    #[tokio::test]
    async fn test_search_returns_resources() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create some resources
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient");
        let result = backend.search(&tenant, &query).await.unwrap();

        assert_eq!(result.resources.items.len(), 2);
    }

    #[tokio::test]
    async fn test_search_count() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient");
        let count = backend.search_count(&tenant, &query).await.unwrap();

        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_search_tenant_isolation() {
        let backend = create_test_backend();

        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        backend
            .create(&tenant1, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant2, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant2, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient");

        let result1 = backend.search(&tenant1, &query).await.unwrap();
        assert_eq!(result1.resources.items.len(), 1);

        let result2 = backend.search(&tenant2, &query).await.unwrap();
        assert_eq!(result2.resources.items.len(), 2);
    }

    // ========================================================================
    // Cursor Pagination Tests
    // ========================================================================

    #[tokio::test]
    async fn test_cursor_pagination_basic() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create 5 resources
        for i in 0..5 {
            backend
                .create(
                    &tenant,
                    "Patient",
                    json!({"name": format!("Patient{}", i)}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }

        // First page with limit of 2
        let query = SearchQuery::new("Patient").with_count(2);
        let page1 = backend.search(&tenant, &query).await.unwrap();

        assert_eq!(page1.resources.items.len(), 2);
        assert!(page1.resources.page_info.has_next);
        assert!(page1.resources.page_info.next_cursor.is_some());

        // Second page using cursor
        let cursor = page1.resources.page_info.next_cursor.unwrap();
        let query2 = SearchQuery::new("Patient")
            .with_count(2)
            .with_cursor(cursor);
        let page2 = backend.search(&tenant, &query2).await.unwrap();

        assert_eq!(page2.resources.items.len(), 2);
        assert!(page2.resources.page_info.has_next);
        assert!(page2.resources.page_info.has_previous);

        // Third page (last)
        let cursor = page2.resources.page_info.next_cursor.unwrap();
        let query3 = SearchQuery::new("Patient")
            .with_count(2)
            .with_cursor(cursor);
        let page3 = backend.search(&tenant, &query3).await.unwrap();

        assert_eq!(page3.resources.items.len(), 1);
        assert!(!page3.resources.page_info.has_next);
        assert!(page3.resources.page_info.next_cursor.is_none());

        // Verify no overlapping IDs
        let page1_ids: Vec<_> = page1.resources.items.iter().map(|r| r.id()).collect();
        let page2_ids: Vec<_> = page2.resources.items.iter().map(|r| r.id()).collect();
        let page3_ids: Vec<_> = page3.resources.items.iter().map(|r| r.id()).collect();

        for id in &page1_ids {
            assert!(!page2_ids.contains(id), "Page 1 and 2 should not overlap");
            assert!(!page3_ids.contains(id), "Page 1 and 3 should not overlap");
        }
        for id in &page2_ids {
            assert!(!page3_ids.contains(id), "Page 2 and 3 should not overlap");
        }
    }

    #[tokio::test]
    async fn test_cursor_pagination_no_more_results() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create 3 resources
        for _ in 0..3 {
            backend
                .create(&tenant, "Patient", json!({}), FhirVersion::default())
                .await
                .unwrap();
        }

        // Request more than available
        let query = SearchQuery::new("Patient").with_count(10);
        let result = backend.search(&tenant, &query).await.unwrap();

        assert_eq!(result.resources.items.len(), 3);
        assert!(!result.resources.page_info.has_next);
        assert!(result.resources.page_info.next_cursor.is_none());
    }

    #[tokio::test]
    async fn test_cursor_pagination_empty() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let query = SearchQuery::new("Patient").with_count(10);
        let result = backend.search(&tenant, &query).await.unwrap();

        assert!(result.resources.items.is_empty());
        assert!(!result.resources.page_info.has_next);
        assert!(!result.resources.page_info.has_previous);
    }

    // ========================================================================
    // MultiTypeSearchProvider Tests
    // ========================================================================

    #[tokio::test]
    async fn test_search_multi_all_types() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create different resource types
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Encounter", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Search all types (empty list)
        let query = SearchQuery::new("Patient"); // Type in query doesn't matter for multi
        let result = backend.search_multi(&tenant, &[], &query).await.unwrap();

        // Should find all 4 resources
        assert_eq!(result.resources.items.len(), 4);
    }

    #[tokio::test]
    async fn test_search_multi_specific_types() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create different resource types
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant, "Encounter", json!({}), FhirVersion::default())
            .await
            .unwrap();

        // Search only Patient and Observation
        let query = SearchQuery::new("Patient");
        let result = backend
            .search_multi(&tenant, &["Patient", "Observation"], &query)
            .await
            .unwrap();

        // Should find 3 resources
        assert_eq!(result.resources.items.len(), 3);

        // Verify types
        let types: Vec<&str> = result
            .resources
            .items
            .iter()
            .map(|r| r.resource_type())
            .collect();
        assert!(types.contains(&"Patient"));
        assert!(types.contains(&"Observation"));
        assert!(!types.contains(&"Encounter"));
    }

    #[tokio::test]
    async fn test_search_multi_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        backend
            .create(&tenant1, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant2, "Patient", json!({}), FhirVersion::default())
            .await
            .unwrap();
        backend
            .create(&tenant2, "Observation", json!({}), FhirVersion::default())
            .await
            .unwrap();

        let query = SearchQuery::new("Patient");

        let result1 = backend.search_multi(&tenant1, &[], &query).await.unwrap();
        assert_eq!(result1.resources.items.len(), 1);

        let result2 = backend.search_multi(&tenant2, &[], &query).await.unwrap();
        assert_eq!(result2.resources.items.len(), 2);
    }

    // ========================================================================
    // IncludeProvider Tests
    // ========================================================================

    #[tokio::test]
    async fn test_resolve_includes_basic() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a patient
        let _patient = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "name": [{"family": "Smith"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create an observation that references the patient
        let observation = backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o1",
                    "subject": {"reference": "Patient/p1"},
                    "code": {"text": "Blood pressure"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Resolve includes for the observation
        let include = IncludeDirective {
            include_type: crate::types::IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: None,
            iterate: false,
        };

        let included = backend
            .resolve_includes(&tenant, &[observation], &[include])
            .await
            .unwrap();

        // Should include the patient
        assert_eq!(included.len(), 1);
        assert_eq!(included[0].resource_type(), "Patient");
        assert_eq!(included[0].id(), "p1");
    }

    #[tokio::test]
    async fn test_resolve_includes_with_target_type_filter() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create resources
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Practitioner",
                json!({"id": "pr1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let observation = backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o1",
                    "subject": {"reference": "Patient/p1"},
                    "performer": [{"reference": "Practitioner/pr1"}]
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Include only Patient references
        let include = IncludeDirective {
            include_type: crate::types::IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: Some("Patient".to_string()),
            iterate: false,
        };

        let included = backend
            .resolve_includes(&tenant, &[observation], &[include])
            .await
            .unwrap();

        assert_eq!(included.len(), 1);
        assert_eq!(included[0].resource_type(), "Patient");
    }

    #[tokio::test]
    async fn test_resolve_includes_empty_resources() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let include = IncludeDirective {
            include_type: crate::types::IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: None,
            iterate: false,
        };

        let included = backend
            .resolve_includes(&tenant, &[], &[include])
            .await
            .unwrap();

        assert!(included.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_includes_tenant_isolation() {
        let backend = create_test_backend();
        let tenant1 =
            TenantContext::new(TenantId::new("tenant-1"), TenantPermissions::full_access());
        let tenant2 =
            TenantContext::new(TenantId::new("tenant-2"), TenantPermissions::full_access());

        // Create patient in tenant 1
        backend
            .create(
                &tenant1,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create observation in tenant 2 that "references" patient in tenant 1
        let observation = backend
            .create(
                &tenant2,
                "Observation",
                json!({
                    "id": "o1",
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let include = IncludeDirective {
            include_type: crate::types::IncludeType::Include,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: None,
            iterate: false,
        };

        // Should NOT include the patient from tenant 1
        let included = backend
            .resolve_includes(&tenant2, &[observation], &[include])
            .await
            .unwrap();

        assert!(included.is_empty());
    }

    // ========================================================================
    // RevincludeProvider Tests
    // ========================================================================

    #[tokio::test]
    async fn test_resolve_revincludes_basic() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Create a patient
        let patient = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create observations that reference the patient
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o1",
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o2",
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Also create an observation for a different patient
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o3",
                    "subject": {"reference": "Patient/p2"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let revinclude = IncludeDirective {
            include_type: crate::types::IncludeType::Revinclude,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: None,
            iterate: false,
        };

        let included = backend
            .resolve_revincludes(&tenant, &[patient], &[revinclude])
            .await
            .unwrap();

        // Should include 2 observations
        assert_eq!(included.len(), 2);
        assert!(included.iter().all(|r| r.resource_type() == "Observation"));
        let ids: Vec<&str> = included.iter().map(|r| r.id()).collect();
        assert!(ids.contains(&"o1"));
        assert!(ids.contains(&"o2"));
    }

    #[tokio::test]
    async fn test_resolve_revincludes_empty() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let patient = backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let revinclude = IncludeDirective {
            include_type: crate::types::IncludeType::Revinclude,
            source_type: "Observation".to_string(),
            search_param: "subject".to_string(),
            target_type: None,
            iterate: false,
        };

        // No observations exist
        let included = backend
            .resolve_revincludes(&tenant, &[patient], &[revinclude])
            .await
            .unwrap();

        assert!(included.is_empty());
    }

    // ========================================================================
    // ChainedSearchProvider Tests
    // ========================================================================

    #[tokio::test]
    async fn test_resolve_chain_simple() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create patients
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "name": [{"family": "Smith"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p2", "name": [{"family": "Jones"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create observations
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o1",
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o2",
                    "subject": {"reference": "Patient/p2"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Manually insert search index entries since FHIRPath extraction
        // may not fully populate them due to unsupported functions
        {
            let conn = backend.get_connection().unwrap();
            // Insert patient names
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_string)
                 VALUES (?1, 'Patient', 'p1', 'name', 'Smith')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_string)
                 VALUES (?1, 'Patient', 'p2', 'name', 'Jones')",
                params![tenant_id],
            ).unwrap();
            // Insert observation subject references
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'Observation', 'o1', 'subject', 'Patient/p1')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'Observation', 'o2', 'subject', 'Patient/p2')",
                params![tenant_id],
            ).unwrap();
        }

        // Find observations where patient.name contains "Smith"
        let matching_ids = backend
            .resolve_chain(&tenant, "Observation", "subject.name", "Smith")
            .await
            .unwrap();

        assert_eq!(matching_ids.len(), 1);
        assert!(matching_ids.contains(&"o1".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_chain_no_match() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create patient
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "name": [{"family": "Smith"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create observation
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o1",
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Manually insert search index entries
        {
            let conn = backend.get_connection().unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_string)
                 VALUES (?1, 'Patient', 'p1', 'name', 'Smith')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'Observation', 'o1', 'subject', 'Patient/p1')",
                params![tenant_id],
            ).unwrap();
        }

        // Search for non-existent name
        let matching_ids = backend
            .resolve_chain(&tenant, "Observation", "subject.name", "Nonexistent")
            .await
            .unwrap();

        assert!(matching_ids.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_reverse_chain() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create patients
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p2"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create observations with codes
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o1",
                    "subject": {"reference": "Patient/p1"},
                    "code": {"coding": [{"code": "8867-4"}]}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                &tenant,
                "Observation",
                json!({
                    "id": "o2",
                    "subject": {"reference": "Patient/p2"},
                    "code": {"coding": [{"code": "other"}]}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Manually insert search index entries since FHIRPath extraction
        // may not fully populate them due to unsupported functions
        {
            let conn = backend.get_connection().unwrap();
            // Insert subject references for observations
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'Observation', 'o1', 'subject', 'Patient/p1')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'Observation', 'o2', 'subject', 'Patient/p2')",
                params![tenant_id],
            ).unwrap();
            // Insert code tokens for observations
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_code)
                 VALUES (?1, 'Observation', 'o1', 'code', '8867-4')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_code)
                 VALUES (?1, 'Observation', 'o2', 'code', 'other')",
                params![tenant_id],
            ).unwrap();
        }

        // _has:Observation:subject:code=8867-4
        let reverse_chain = ReverseChainedParameter::terminal(
            "Observation",
            "subject",
            "code",
            crate::types::SearchValue::eq("8867-4"),
        );

        let matching_ids = backend
            .resolve_reverse_chain(&tenant, "Patient", &reverse_chain)
            .await
            .unwrap();

        // Should find p1 (referenced by observation with code 8867-4)
        assert_eq!(matching_ids.len(), 1);
        assert!(matching_ids.contains(&"p1".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_chain_multi_level() {
        // Test 3-level chain: Observation?subject.organization.name=Hospital
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create organization
        backend
            .create(
                &tenant,
                "Organization",
                json!({"id": "org1", "name": "General Hospital"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create patient linked to organization
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "managingOrganization": {"reference": "Organization/org1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create observation linked to patient
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1", "subject": {"reference": "Patient/p1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Manually insert search index entries
        {
            let conn = backend.get_connection().unwrap();
            // Organization name
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_string)
                 VALUES (?1, 'Organization', 'org1', 'name', 'General Hospital')",
                params![tenant_id],
            ).unwrap();
            // Patient -> Organization reference
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'Patient', 'p1', 'organization', 'Organization/org1')",
                params![tenant_id],
            ).unwrap();
            // Observation -> Patient reference
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'Observation', 'o1', 'subject', 'Patient/p1')",
                params![tenant_id],
            ).unwrap();
        }

        // Find observations where patient's organization name contains "Hospital"
        let matching_ids = backend
            .resolve_chain(
                &tenant,
                "Observation",
                "subject.organization.name",
                "Hospital",
            )
            .await
            .unwrap();

        assert_eq!(matching_ids.len(), 1);
        assert!(matching_ids.contains(&"o1".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_chain_with_type_modifier() {
        // Test chain with explicit type: Observation?subject:Patient.name=Smith
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create patient
        backend
            .create(
                &tenant,
                "Patient",
                json!({"id": "p1", "name": [{"family": "Smith"}]}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Create observation
        backend
            .create(
                &tenant,
                "Observation",
                json!({"id": "o1", "subject": {"reference": "Patient/p1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Manually insert search index entries
        {
            let conn = backend.get_connection().unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_string)
                 VALUES (?1, 'Patient', 'p1', 'name', 'Smith')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'Observation', 'o1', 'subject', 'Patient/p1')",
                params![tenant_id],
            ).unwrap();
        }

        // Use explicit type modifier
        let matching_ids = backend
            .resolve_chain(&tenant, "Observation", "subject:Patient.name", "Smith")
            .await
            .unwrap();

        assert_eq!(matching_ids.len(), 1);
        assert!(matching_ids.contains(&"o1".to_string()));
    }

    #[tokio::test]
    async fn test_chain_invalid_param_error() {
        // Test that invalid chain parameters return an error
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // Try a chain with non-existent parameter
        let result = backend
            .resolve_chain(&tenant, "Observation", "invalid.param", "value")
            .await;

        // Should return an error due to unknown parameter
        assert!(result.is_err());
    }

    // ========================================================================
    // Helper Method Tests
    // ========================================================================

    #[test]
    fn test_parse_reference_simple() {
        let backend = SqliteBackend::in_memory().unwrap();

        let result = backend.parse_reference("Patient/123");
        assert_eq!(result, Some(("Patient".to_string(), "123".to_string())));
    }

    #[test]
    fn test_parse_reference_url() {
        let backend = SqliteBackend::in_memory().unwrap();

        let result = backend.parse_reference("http://example.com/fhir/Patient/456");
        assert_eq!(result, Some(("Patient".to_string(), "456".to_string())));
    }

    // ========================================================================
    // Token Search with system|code Tests
    // ========================================================================

    #[tokio::test]
    async fn test_token_search_system_and_code() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create two DocumentReferences with different type codes
        backend
            .create(
                &tenant,
                "DocumentReference",
                json!({
                    "resourceType": "DocumentReference",
                    "id": "doc1",
                    "status": "current",
                    "type": {
                        "coding": [{
                            "system": "http://loinc.org",
                            "code": "86533-7",
                            "display": "Patient Living will"
                        }]
                    },
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "DocumentReference",
                json!({
                    "resourceType": "DocumentReference",
                    "id": "doc2",
                    "status": "current",
                    "type": {
                        "coding": [{
                            "system": "http://loinc.org",
                            "code": "34117-2",
                            "display": "History and physical note"
                        }]
                    },
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Insert token search index entries for both documents
        {
            let conn = backend.get_connection().unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_system, value_token_code)
                 VALUES (?1, 'DocumentReference', 'doc1', 'type', 'http://loinc.org', '86533-7')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_system, value_token_code)
                 VALUES (?1, 'DocumentReference', 'doc2', 'type', 'http://loinc.org', '34117-2')",
                params![tenant_id],
            ).unwrap();
        }

        // Search with system|code: should find only doc1
        let mut query = SearchQuery::new("DocumentReference");
        query.parameters.push(SearchParameter {
            name: "type".to_string(),
            param_type: crate::types::SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://loinc.org|86533-7")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();

        assert_eq!(
            result.resources.items.len(),
            1,
            "Should find exactly 1 DocumentReference with type http://loinc.org|86533-7"
        );
        assert_eq!(result.resources.items[0].id(), "doc1");
    }

    #[tokio::test]
    async fn test_token_search_code_only() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create a DocumentReference
        backend
            .create(
                &tenant,
                "DocumentReference",
                json!({
                    "resourceType": "DocumentReference",
                    "id": "doc1",
                    "status": "current",
                    "type": {
                        "coding": [{
                            "system": "http://loinc.org",
                            "code": "86533-7"
                        }]
                    }
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Insert token search index entry
        {
            let conn = backend.get_connection().unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_system, value_token_code)
                 VALUES (?1, 'DocumentReference', 'doc1', 'type', 'http://loinc.org', '86533-7')",
                params![tenant_id],
            ).unwrap();
        }

        // Search with code only (no system): should still find doc1
        let mut query = SearchQuery::new("DocumentReference");
        query.parameters.push(SearchParameter {
            name: "type".to_string(),
            param_type: crate::types::SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("86533-7")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();

        assert_eq!(
            result.resources.items.len(),
            1,
            "Code-only search should find the document regardless of system"
        );
        assert_eq!(result.resources.items[0].id(), "doc1");
    }

    #[tokio::test]
    async fn test_token_search_wrong_system() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create a DocumentReference
        backend
            .create(
                &tenant,
                "DocumentReference",
                json!({
                    "resourceType": "DocumentReference",
                    "id": "doc1",
                    "status": "current"
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Insert token with loinc.org system
        {
            let conn = backend.get_connection().unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_system, value_token_code)
                 VALUES (?1, 'DocumentReference', 'doc1', 'type', 'http://loinc.org', '86533-7')",
                params![tenant_id],
            ).unwrap();
        }

        // Search with wrong system: should return 0
        let mut query = SearchQuery::new("DocumentReference");
        query.parameters.push(SearchParameter {
            name: "type".to_string(),
            param_type: crate::types::SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://snomed.info/sct|86533-7")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();

        assert_eq!(
            result.resources.items.len(),
            0,
            "Search with wrong system should return no results"
        );
    }

    #[tokio::test]
    async fn test_token_search_combined_with_reference() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let tenant_id = tenant.tenant_id().as_str();

        // Create two DocumentReferences for different patients
        backend
            .create(
                &tenant,
                "DocumentReference",
                json!({
                    "resourceType": "DocumentReference",
                    "id": "doc1",
                    "status": "current",
                    "type": {
                        "coding": [{"system": "http://loinc.org", "code": "86533-7"}]
                    },
                    "subject": {"reference": "Patient/p1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        backend
            .create(
                &tenant,
                "DocumentReference",
                json!({
                    "resourceType": "DocumentReference",
                    "id": "doc2",
                    "status": "current",
                    "type": {
                        "coding": [{"system": "http://loinc.org", "code": "86533-7"}]
                    },
                    "subject": {"reference": "Patient/p2"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Insert search index entries for both
        {
            let conn = backend.get_connection().unwrap();
            // Type tokens
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_system, value_token_code)
                 VALUES (?1, 'DocumentReference', 'doc1', 'type', 'http://loinc.org', '86533-7')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_token_system, value_token_code)
                 VALUES (?1, 'DocumentReference', 'doc2', 'type', 'http://loinc.org', '86533-7')",
                params![tenant_id],
            ).unwrap();
            // Patient references
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'DocumentReference', 'doc1', 'patient', 'Patient/p1')",
                params![tenant_id],
            ).unwrap();
            conn.execute(
                "INSERT INTO search_index (tenant_id, resource_type, resource_id, param_name, value_reference)
                 VALUES (?1, 'DocumentReference', 'doc2', 'patient', 'Patient/p2')",
                params![tenant_id],
            ).unwrap();
        }

        // Search with both patient AND type (system|code)
        let mut query = SearchQuery::new("DocumentReference");
        query.parameters.push(SearchParameter {
            name: "patient".to_string(),
            param_type: crate::types::SearchParamType::Reference,
            modifier: None,
            values: vec![SearchValue::eq("p1")],
            chain: vec![],
            components: vec![],
        });
        query.parameters.push(SearchParameter {
            name: "type".to_string(),
            param_type: crate::types::SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://loinc.org|86533-7")],
            chain: vec![],
            components: vec![],
        });

        let result = backend.search(&tenant, &query).await.unwrap();

        assert_eq!(
            result.resources.items.len(),
            1,
            "Combined patient + type search should find exactly doc1"
        );
        assert_eq!(result.resources.items[0].id(), "doc1");
    }
}
