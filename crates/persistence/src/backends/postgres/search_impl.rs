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
    ChainedSearchProvider, IncludeProvider, MultiTypeSearchProvider, ResourceStorage,
    RevincludeProvider, SearchProvider, SearchResult, TextSearchProvider,
};
use crate::error::{BackendError, QueryErrorExt, SearchError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    CursorDirection, CursorValue, IncludeDirective, Page, PageCursor, PageInfo, Pagination,
    ReverseChainedParameter, SearchQuery, StoredResource,
};

use super::PostgresBackend;
use super::cached::{query_dyn_cached, query_one_dyn_cached};
use super::search::chain_builder::ChainQueryBuilder;
use super::search::query_builder::{PostgresQueryBuilder, SortValueKind, SqlParam};

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "postgres".to_string(),
        message,
        source: None,
    })
}

/// Whether a built search statement's text is drawn from a bounded family, and
/// so may be kept in the connection's prepared-statement cache.
///
/// Everything the query builder varies is bounded except one thing. The
/// predicate shape is bounded by the parameter types; the inlined `param_name`
/// is bounded by the SearchParameter registry; the inlined `LIMIT` is bounded
/// because `_count` is clamped to 1000 by `extractors::pagination`. `OFFSET` is
/// not bounded by anything, and it is inlined too — so an offset-paged crawl
/// mints one statement per page, for as long as the client keeps paging. Those
/// stay off the cached path rather than rely on the cache's flush to clean up
/// after them.
///
/// Cursor pagination is unaffected: it binds its keyset values as parameters, so
/// every page of a cursor-paged search shares one statement text.
fn statement_is_reusable(query: &SearchQuery) -> bool {
    query.offset.unwrap_or(0) == 0
}

fn reject_contained_missing(query: &SearchQuery) -> StorageResult<()> {
    if query.contained != crate::types::ContainedMode::Off
        && query
            .parameters
            .iter()
            .any(|param| matches!(param.modifier, Some(crate::types::SearchModifier::Missing)))
    {
        return Err(StorageError::Search(SearchError::QueryParseError {
            message: "PostgreSQL does not support :missing with _contained=true or both"
                .to_string(),
        }));
    }
    Ok(())
}

/// Decides whether a page can be resolved from `search_index` alone, returning
/// the predicate to resolve it with.
///
/// Gates, and why each one is load-bearing:
/// - `Denormalized` layout: on a legacy database the rows carry no sort key, so
///   the fast path would order by NULL and return an arbitrary page.
/// - no cursor, no offset: both paginate against `resources`.
/// - default sort: any other sort key is not on the index row.
/// - exactly one membership test: a conjunction needs rows that jointly satisfy
///   it, which a single-row predicate cannot express.
fn fast_index_pred(
    query: &SearchQuery,
    filter_sql: Option<&str>,
    layout: super::schema::IndexLayout,
    has_cursor: bool,
) -> Option<String> {
    if has_cursor
        || query.offset.is_some()
        || !query.sort.is_empty()
        || layout != super::schema::IndexLayout::Denormalized
    {
        return None;
    }
    PostgresQueryBuilder::single_index_predicate(filter_sql?).map(str::to_string)
}

/// Whether the fast path's ordered scan needs a guard against an empty match
/// set.
///
/// # The defect this closes
///
/// The fast path takes its page from `idx_search_quantity_recent`, which is
/// keyed `(tenant_id, resource_type, param_name, last_updated DESC,
/// resource_id)` — the scan streams the parameter slice newest-first and
/// filters `value_quantity_value` from the payload as it goes, so the `LIMIT`
/// can stop after 21 matches. Costing that plan needs the *selectivity* of the
/// value predicate, and Postgres has only a table-wide histogram of
/// `value_quantity_value` — one column shared by every `param_name` of every
/// resource type. On the benchmark corpus `Observation.value-quantity` tops out
/// at 54,786 while `component-value-quantity` rows in the same column reach
/// 995,710, so `value_quantity_value >= 99999.5` is estimated at a fraction of
/// a percent of the slice when the true count is **zero**. The planner prices
/// the ordered scan as "read a few thousand entries and stop"; it walks all
/// 512,311 entries of the slice and returns nothing.
///
/// Measured on run 33213565802: 199.5 s across 23,847 calls — **25.9 % of the
/// whole search suite's Postgres execution time** — in the single statement
/// ending `AND (value_quantity_value >= $3) ORDER BY last_updated DESC …
/// LIMIT 21`, against 15.7 s across 23,823 calls for the two-sided `eq` form of
/// the same parameter, which Postgres estimates correctly and serves from the
/// value-first `idx_search_quantity`. 4.65 % of `value-quantity` requests took
/// over 100 ms (p50 5.3 ms, p99 199.9 ms, max 438 ms), uniformly across the
/// run, and the two `(value, prefix)` combinations `k6/searchConfig.js` issues
/// with zero true matches — `gt 99999`, `ge 99999` — are 2 of its 42
/// combinations, i.e. 4.76 %.
///
/// # Why a guard rather than an index or a statistics change
///
/// A one-sided range cannot be served by any single index at both densities:
/// value-first reads only the matches but must sort them all to answer
/// `ORDER BY last_updated` (500k rows for a broad range), recent-first is
/// already ordered but must walk the slice until the `LIMIT` fills. Both
/// indexes exist (v19/v20) precisely so the planner can choose per query, and
/// that remains right — the estimate is what is wrong, not the index set.
///
/// So this adds an **uncorrelated `EXISTS` over the same predicate**. Postgres
/// cannot pull an uncorrelated `EXISTS` up into a semi-join, so it becomes an
/// InitPlan evaluated once, before the scan, against the value-first index:
/// a single seek. When it is false the ordered scan is gated off entirely by a
/// `One-Time Filter` and the whole statement costs one index descent instead of
/// a full slice walk. When it is true the guard added one seek and changed
/// nothing else.
///
/// It is a tautology given the outer predicate — the outer `WHERE` already
/// requires a row satisfying it, so `EXISTS` over the identical predicate can
/// only be false when the result is empty. No row can be added or removed.
///
/// It does **not** fix the sparse-but-non-empty case (say 5 matches in 512,311
/// rows), which is the same misestimate with a smaller multiplier; that needs
/// per-`param_name` statistics on `value_quantity_value`, which is a schema
/// change.
///
/// # Scope
///
/// Quantity and number only, and only for an open-ended comparator. Both are
/// built by `numeric_predicate` and both read a column whose value range varies
/// by orders of magnitude between parameters sharing it.
///
/// Not `date`, which has the same query shape and is measurably unaffected:
/// every date parameter in the table shares one calendar range, so the pooled
/// `value_date` histogram is a good proxy for each slice. The benchmark issues
/// `Observation?date=gt2070-01-01T00:00:00` and `Patient?birthdate=gt2070-01-01`
/// — both zero-match, one over a 689,080-row slice — and both stay at p99
/// 17 ms because the planner correctly estimates zero and picks the value-first
/// index. The same latent failure exists for date if a deployment ever mixes
/// wildly different date ranges under one column, but it is not present here
/// and a guard is not free.
///
/// Not equality forms (token, reference, uri, `_id`): v21/v27 put the value
/// ahead of the sort key in those indexes, so the scan seeks straight to the
/// value and the `LIMIT` stops after 21 whatever the selectivity. Selectivity
/// stops mattering, and there is nothing to guard.
fn open_range_needs_empty_guard(query: &SearchQuery) -> bool {
    let [param] = query.parameters.as_slice() else {
        return false;
    };
    if !matches!(
        param.param_type,
        crate::types::SearchParamType::Quantity | crate::types::SearchParamType::Number
    ) {
        return false;
    }
    param.values.iter().any(|v| {
        matches!(
            v.prefix,
            crate::types::SearchPrefix::Gt
                | crate::types::SearchPrefix::Ge
                | crate::types::SearchPrefix::Lt
                | crate::types::SearchPrefix::Le
                | crate::types::SearchPrefix::Sa
                | crate::types::SearchPrefix::Eb
        )
    })
}

impl PostgresBackend {
    /// The body of [`SearchProvider::search`], run on a caller-supplied client.
    ///
    /// `SearchProvider::search` takes a fresh pooled client, which cannot see
    /// rows an open transaction has written. A bundle entry that must resolve
    /// `ifNoneExist` against what earlier entries in the same transaction wrote
    /// runs this on the transaction's own client instead (#511). `total` is
    /// computed by the caller so the count query's client is not held across
    /// this one's await points.
    pub(crate) async fn search_with_client(
        &self,
        client: &deadpool_postgres::Client,
        tenant: &TenantContext,
        query: &SearchQuery,
        total: Option<u64>,
    ) -> StorageResult<SearchResult> {
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

        let search_filter = if !query.parameters.is_empty() || query.compartment.is_some() {
            PostgresQueryBuilder::build_search_query_for(query, param_offset, self.index_layout())
        } else {
            None
        };
        // v18 fast path: resolve the page from `search_index` alone.
        //
        // The sort key lives on every index row (see `migrate_v16_to_v17`), so a
        // single-parameter search can take its top-n before touching `resources`
        // and then fetch only the rows it returns. The general form below has to
        // join every match first — 42,927 whole resources for a 21-row page on
        // the benchmark dataset — because the planner cannot estimate a range
        // predicate whose selectivity is conditional on `param_name`.
        let fast_index_pred = fast_index_pred(
            query,
            search_filter.as_ref().map(|f| f.sql.as_str()),
            self.index_layout(),
            cursor.is_some(),
        );

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
        let (sql, has_previous) = if let Some(pred) = &fast_index_pred {
            // `keyset` is always `Some` here: the fast path requires the default
            // sort, which yields the `last_updated` keyset.
            // Gate the ordered scan on the predicate matching anything at all;
            // see `open_range_needs_empty_guard`. The subquery is uncorrelated,
            // so it is an InitPlan run once against the value-first index, and
            // its unqualified column references bind to its own `g`.
            let guard = if open_range_needs_empty_guard(query) {
                format!(
                    " AND EXISTS (SELECT 1 FROM search_index g \
                                  WHERE g.tenant_id = $1 AND g.resource_type = $2 AND {pred})"
                )
            } else {
                String::new()
            };
            let sql = format!(
                "SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version, \
                        r.last_updated AS sort_key \
                 FROM ( \
                   SELECT DISTINCT resource_id, last_updated FROM search_index \
                   WHERE tenant_id = $1 AND resource_type = $2 AND {pred}{guard} \
                   ORDER BY last_updated DESC, resource_id ASC LIMIT {lim} \
                 ) c \
                 JOIN resources r \
                   ON r.tenant_id = $1 AND r.resource_type = $2 AND r.id = c.resource_id \
                 WHERE r.is_deleted = FALSE \
                 ORDER BY c.last_updated DESC, c.resource_id ASC",
                pred = pred,
                guard = guard,
                lim = count + 1,
            );
            (sql, false)
        } else if let (Some(cursor), Some(k)) = (&cursor, &keyset) {
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
        let rows = if statement_is_reusable(query) {
            query_dyn_cached(client, &sql, &param_refs).await
        } else {
            client.query(&sql, &param_refs).await
        }
        .or_query_error("Failed to execute search")?;

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

        // `total` was computed up-front (before acquiring `client`) to avoid
        // holding a non-Send guard across the count query's await.
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
            scores: Default::default(),
        })
    }
}

#[async_trait]
impl SearchProvider for PostgresBackend {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        reject_contained_missing(query)?;

        // `_contained` search uses a dedicated path (different index columns and
        // heterogeneous result types); standard search handles `_contained=false`.
        if query.contained != crate::types::ContainedMode::Off {
            return self.search_contained(tenant, query).await;
        }

        // Populate Bundle.total only when the client asked for it
        // (`_total=accurate|estimate`). Computed up-front so the count query's
        // client is not held across the main query's await points.
        let total = if query.wants_total() {
            Some(self.search_count(tenant, query).await?)
        } else {
            None
        };

        let client = self.get_client().await?;
        self.search_with_client(&client, tenant, query, total).await
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        reject_contained_missing(query)?;

        let client = self.get_client().await?;
        let tenant_id = tenant.tenant_id().as_str();
        let resource_type = &query.resource_type;

        let (sql, params): (
            String,
            Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>>,
        ) = if !query.parameters.is_empty() || query.compartment.is_some() {
            let filter =
                PostgresQueryBuilder::build_search_query_for(query, 2, self.index_layout());

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

        // No `LIMIT`/`OFFSET` in this one at all: the text varies only by
        // predicate shape and inlined `param_name`, so it is unconditionally
        // reusable.
        let row = query_one_dyn_cached(&client, &sql, &param_refs)
            .await
            .or_query_error("Failed to count resources")?;

        let count: i64 = row.get(0);
        Ok(count as u64)
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

        // Same `OFFSET` reservation as `search`; the type list is inlined but is
        // bounded by the resource types a client can name.
        let rows = if offset == 0 {
            query_dyn_cached(&client, &sql, &[&tenant_id]).await
        } else {
            client.query(&sql, &[&tenant_id]).await
        }
        .or_query_error("Failed to execute multi-type search")?;

        let mut resources = Vec::new();
        for row in &rows {
            let res_type: String = row.get(0);
            let id: String = row.get(1);
            let version_id: String = row.get(2);
            let json_data: serde_json::Value = row.get(3);
            let last_updated: chrono::DateTime<Utc> = row.get(4);
            let fhir_version_str: String = row.get(5);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

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
            scores: Default::default(),
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

            // Deliberately NOT cached. `placeholders` has one entry per
            // reference value, and there are two per resource on the page, so
            // the statement text is a function of the page's size — up to 2,000
            // placeholders, a distinct text for every page width a client asks
            // for, and a large one. That is precisely the unbounded key the
            // statement cache must not be fed. It also runs once per search
            // rather than once per result, so there is little to win.
            let rows = client
                .query(&sql, &param_refs)
                .await
                .or_query_error("Failed to execute revinclude query")?;

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
        let builder = ChainQueryBuilder::new(tenant_id, base_type, self.tenant_registry(tenant_id))
            .with_param_offset(1);
        let parsed = builder
            .parse_chain(chain)
            .map_err(|e| internal_error(format!("Failed to parse chain: {}", e)))?;
        // Strip the comparator prefix only when the terminal parameter's type
        // admits one: dates/numbers/quantities compare, but a string value
        // like `family=Levine` must never be misread as le + "vine" (#258).
        let candidate = crate::types::SearchValue::parse(value);
        let parsed_value = if candidate.prefix != crate::types::SearchPrefix::Eq
            && candidate.prefix.is_valid_for(parsed.terminal_type)
        {
            candidate
        } else {
            crate::types::SearchValue::eq(value)
        };
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

        // Deliberately NOT cached: `chain_builder` splices a token's *system*
        // into the SQL as a literal (search/chain_builder.rs, the `Token` arms of
        // `build_terminal_condition` and its reverse twin). The value is
        // quote-escaped, so this is a cache-key problem rather than an injection
        // one — but a client-supplied value in the text means a distinct
        // statement per system, which is exactly the unbounded key to avoid.
        // Binding it instead means renumbering the chain builder's placeholder
        // accounting, which another seat is already inside; recorded rather than
        // fixed here.
        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to execute chain query")?;

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
        let builder = ChainQueryBuilder::new(tenant_id, base_type, self.tenant_registry(tenant_id))
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

        // Not cached, for the reason given in `resolve_chain`.
        let rows = client
            .query(&sql, &param_refs)
            .await
            .or_query_error("Failed to execute reverse chain query")?;

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

        let rows = query_dyn_cached(&client, &sql, &[&tenant_id, &resource_type, &text])
            .await
            .or_query_error("Failed to execute text search")?;

        let mut resources = Vec::new();
        for row in &rows {
            let id: String = row.get(0);
            let version_id: String = row.get(1);
            let json_data: serde_json::Value = row.get(2);
            let last_updated: chrono::DateTime<Utc> = row.get(3);
            let fhir_version_str: String = row.get(4);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

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
            scores: Default::default(),
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

        let rows = query_dyn_cached(&client, &sql, &[&tenant_id, &resource_type, &content])
            .await
            .or_query_error("Failed to execute content search")?;

        let mut resources = Vec::new();
        for row in &rows {
            let id: String = row.get(0);
            let version_id: String = row.get(1);
            let json_data: serde_json::Value = row.get(2);
            let last_updated: chrono::DateTime<Utc> = row.get(3);
            let fhir_version_str: String = row.get(4);

            let fhir_version = FhirVersion::from_storage(&fhir_version_str)
                .unwrap_or_else(helios_fhir::FhirVersion::default_enabled);

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
            scores: Default::default(),
        })
    }
}

/// Finds the `contained[]` entry with the given local `id` in a container's
/// content.
fn extract_contained_resource(
    content: &serde_json::Value,
    local_id: &str,
) -> Option<serde_json::Value> {
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
    content: serde_json::Value,
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

// Contained (`_contained`) search.
impl PostgresBackend {
    /// Executes a `_contained=true|both` search. See the SQLite backend's
    /// `search_contained` for the shared semantics: matches contained resources
    /// of `query.resource_type` via the `is_contained` index rows, returns the
    /// containers (default) or the contained resources (`_containedType=contained`),
    /// and for `both` merges top-level matches first. Paginated by
    /// `_offset`/`_count` as a single window (no keyset cursor).
    async fn search_contained(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        use crate::types::{ContainedMode, ContainedReturn};

        let tenant_id = tenant.tenant_id().as_str();
        let contained_type = query.resource_type.as_str();

        // 1. Resolve contained matches → (container_type, container_id, local_id).
        let matches: Vec<(String, String, Option<String>)> =
            match PostgresQueryBuilder::build_contained(query) {
                Some(fragment) => {
                    let client = self.get_client().await?;
                    let mut params: Vec<Box<dyn tokio_postgres::types::ToSql + Sync + Send>> = vec![
                        Box::new(tenant_id.to_string()),
                        Box::new(contained_type.to_string()),
                    ];
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
                    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
                        .iter()
                        .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
                        .collect();
                    let rows = query_dyn_cached(&client, &fragment.sql, &param_refs)
                        .await
                        .or_query_error("Failed to execute contained query")?;
                    rows.iter()
                        .map(|row| {
                            (
                                row.get::<_, String>(0),
                                row.get::<_, String>(1),
                                row.get::<_, Option<String>>(2),
                            )
                        })
                        .collect()
                }
                None => Vec::new(),
            };

        // 2. Materialize result items (container or contained), de-duplicated.
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

        // 3. For `both`, merge top-level matches ahead of contained ones.
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

        // 4. Apply the offset/count window.
        let count = query.count.unwrap_or(100) as usize;
        let offset = query.offset.unwrap_or(0) as usize;
        let total_matches = items.len() as u64;
        let windowed: Vec<StoredResource> = items.into_iter().skip(offset).take(count).collect();

        let total = if query.wants_total() {
            Some(total_matches)
        } else {
            None
        };
        let page = Page::new(windowed, PageInfo::end());
        let mut result = SearchResult::new(page);
        if let Some(t) = total {
            result = result.with_total(t);
        }
        Ok(result)
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
        // One statement per included reference: an `_include` that resolves 20
        // subjects issues this 20 times for one search. Literal text, primary-key
        // lookup — the safest thing in the file to cache, and it was the only
        // hot uncached statement left outside the query builder's output.
        let rows = query_dyn_cached(
            client,
            "SELECT version_id, data, last_updated, fhir_version FROM resources
                 WHERE tenant_id = $1 AND resource_type = $2 AND id = $3 AND is_deleted = FALSE",
            &[&tenant_id, &resource_type, &id],
        )
        .await
        .or_query_error("Failed to fetch resource")?;

        if rows.is_empty() {
            return Ok(None);
        }

        let row = &rows[0];
        let version_id: String = row.get(0);
        let json_data: serde_json::Value = row.get(1);
        let last_updated: chrono::DateTime<Utc> = row.get(2);
        let fhir_version_str: String = row.get(3);
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
}

#[cfg(test)]
mod statement_reuse_tests {
    use super::*;
    use crate::types::SearchQuery;

    fn q() -> SearchQuery {
        SearchQuery::new("Patient")
    }

    #[test]
    fn a_first_page_is_reusable() {
        assert!(statement_is_reusable(&q()));
    }

    #[test]
    fn an_explicit_zero_offset_is_reusable() {
        let mut query = q();
        query.offset = Some(0);
        assert!(statement_is_reusable(&query));
    }

    #[test]
    fn a_nonzero_offset_is_not_reusable() {
        // `_offset` is inlined into the SQL and nothing clamps it, so caching
        // this text would mint one prepared statement per page crawled.
        let mut query = q();
        query.offset = Some(50);
        assert!(!statement_is_reusable(&query));
    }
}

#[cfg(test)]
mod fast_path_tests {
    use super::*;
    use crate::backends::postgres::schema::IndexLayout;
    use crate::types::{
        SearchParamType, SearchParameter, SearchPrefix, SearchValue, SortDirective,
    };

    fn date_query() -> SearchQuery {
        SearchQuery::new("Encounter").with_parameter(SearchParameter {
            name: "date".to_string(),
            param_type: SearchParamType::Date,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Gt, "2010-01-01")],
            chain: vec![],
            components: vec![],
        })
    }

    fn filter_of(query: &SearchQuery) -> String {
        PostgresQueryBuilder::build_search_query(query, 2)
            .expect("condition")
            .sql
    }

    #[test]
    fn taken_for_a_default_sorted_single_parameter_page() {
        let q = date_query();
        let pred = fast_index_pred(&q, Some(&filter_of(&q)), IndexLayout::Denormalized, false);
        assert_eq!(
            pred.as_deref(),
            Some("param_name = 'date' AND value_date >= $3")
        );
    }

    /// The string fast path is the statement v34 rewrote — 30% of the search
    /// suite's Postgres time in run 33128380492 — so pin that its two-parameter
    /// range form still matches `INDEX_MEMBERSHIP_PREFIX` exactly. If it stopped
    /// matching, the page would silently fall back to the join-everything path
    /// and the seek would be worth nothing.
    #[test]
    fn taken_for_the_v33_string_range_form() {
        let q = SearchQuery::new("Patient").with_parameter(SearchParameter {
            name: "address".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::new(SearchPrefix::Eq, "Springfield")],
            chain: vec![],
            components: vec![],
        });
        let pred = fast_index_pred(&q, Some(&filter_of(&q)), IndexLayout::Denormalized, false);
        assert_eq!(
            pred.as_deref(),
            Some(
                "param_name = 'address' AND \
                 COALESCE(value_string_folded, lower(value_string)) ~>=~ $3 AND \
                 COALESCE(value_string_folded, lower(value_string)) ~<~ $4"
            )
        );
    }

    #[test]
    fn refused_on_a_legacy_layout() {
        // The decisive gate: legacy rows have no `last_updated`, so ordering by
        // it would return an arbitrary page rather than the first one.
        let q = date_query();
        assert!(fast_index_pred(&q, Some(&filter_of(&q)), IndexLayout::Legacy, false).is_none());
    }

    #[test]
    fn refused_with_a_cursor_or_offset() {
        let q = date_query();
        let filter = filter_of(&q);
        assert!(fast_index_pred(&q, Some(&filter), IndexLayout::Denormalized, true).is_none());

        let mut q2 = date_query();
        q2.offset = Some(40);
        assert!(fast_index_pred(&q2, Some(&filter), IndexLayout::Denormalized, false).is_none());
    }

    #[test]
    fn refused_for_a_non_default_sort() {
        let mut q = date_query();
        q.sort = vec![SortDirective {
            parameter: "_id".to_string(),
            direction: crate::types::SortDirection::Ascending,
            param_type: Some(SearchParamType::Token),
        }];
        let filter = filter_of(&q);
        assert!(fast_index_pred(&q, Some(&filter), IndexLayout::Denormalized, false).is_none());
    }

    #[test]
    fn refused_without_a_filter() {
        let q = SearchQuery::new("Encounter");
        assert!(fast_index_pred(&q, None, IndexLayout::Denormalized, false).is_none());
    }
}
