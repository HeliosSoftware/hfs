//! Search implementation for SQLite backend.
//!
//! This module provides search functionality for the SQLite backend including:
//! - Basic single-type search
//! - Multi-type search
//! - _include and _revinclude support
//! - `search()` refuses a query whose chains were not resolved first by the
//!   shared resolver (`crate::search::resolve_chains`, #1389)
//! - Search parameter filtering using the search_index table

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::Utc;
use helios_fhir::FhirVersion;
use rusqlite::params;

use crate::core::{
    IncludeProvider, MultiTypeSearchProvider, ResourceStorage, RevincludeProvider, SearchProvider,
    SearchResult,
};
use crate::error::{BackendError, QueryErrorExt, SearchError, StorageError, StorageResult};
use crate::tenant::TenantContext;
use crate::types::{
    CursorDirection, CursorValue, IncludeDirective, Page, PageCursor, PageInfo, SearchQuery,
    SearchValue, StoredResource,
};

use super::SqliteBackend;
use super::search::{
    KeysetKey, QueryBuilder, SortValueKind, SqlParam, is_parameter_rows,
    membership_exists_predicate,
};

fn internal_error(message: String) -> StorageError {
    StorageError::Backend(BackendError::Internal {
        backend_name: "sqlite".to_string(),
        message,
        source: None,
    })
}

/// Rejects `_id` / `_lastUpdated` modifiers their dedicated query builders
/// cannot honour (#1092). Both are dispatched by name in
/// `build_special_parameter_condition`, bypassing the generic per-type
/// modifier handling, so this must run before
/// every path that can reach it: the top-level `search`/`search_count` below,
/// and `search_with_connection` (also reached directly by the in-transaction
/// `ifNoneExist` resolution path, `find_matching_resources_in_tx` in
/// `storage.rs`, which never goes through `search`).
///
/// Every one of those paths must also refuse a date value that is not a date
/// (#1293, #1295), so the shared date gate runs here too: an invalid value is
/// an error, never a query the builder has to make something of.
fn reject_unsupported_metadata_modifier(query: &SearchQuery) -> StorageResult<()> {
    use crate::types::{ContainedMode, SearchModifier, SearchParamType};
    crate::search::reject_unsupported_metadata_modifier(query)?;
    crate::search::validate_date_values(query)?;
    // And a number or quantity value that is not a number (#1319, #1340).
    crate::search::validate_numeric_values(query)?;
    // And a value that is empty, or has an empty alternative: `family=Zzz,`
    // is a prefix match on `""`, which is every family name (#1380).
    crate::search::validate_value_presence(query)?;
    // And a composite value with the wrong number of `$`-separated parts
    // (`code-value-quantity=8302-2`): the builder matched nothing, MongoDB
    // answered 400 (#1236). After the empty-value check, which owns an
    // empty alternative.
    crate::search::validate_composite_values(query)?;
    for parameter in &query.parameters {
        if parameter.param_type != SearchParamType::Composite
            || (query.contained == ContainedMode::Off
                && matches!(parameter.modifier, Some(SearchModifier::Missing)))
        {
            continue;
        }
        for family in parameter
            .components
            .iter()
            .map(|component| component.param_type)
            .collect::<HashSet<_>>()
        {
            let count = parameter
                .components
                .iter()
                .filter(|component| component.param_type == family)
                .count();
            if count > 1
                && (count > u8::MAX as usize
                    || !matches!(
                        family,
                        SearchParamType::Token
                            | SearchParamType::Number
                            | SearchParamType::Quantity
                            | SearchParamType::String
                            | SearchParamType::Date
                    ))
            {
                return Err(StorageError::Search(SearchError::InvalidComposite {
                    message: format!(
                        "search parameter '{}' has unsupported repeated {family:?} components ({count}) on SQLite",
                        parameter.name
                    ),
                }));
            }
        }
    }
    // And a chain nobody resolved (#1389).
    reject_unresolved_chains(query)
}

/// Refuses a query that still carries chained or reverse-chained (`_has`)
/// parameters (#1389). The query builder reads neither: an unresolved `_has`
/// was silently dropped (every resource of the type matched) and a forward
/// chain was read as a plain reference predicate on its first hop. Callers
/// resolve chains first (`crate::search::resolve_chains`, as REST does), which
/// strips both; anything that reaches here with one is an error, never a
/// wrong answer. Runs after the `_contained` refusal so that path keeps its
/// more specific error.
fn reject_unresolved_chains(query: &SearchQuery) -> StorageResult<()> {
    if let Some(param) = query.parameters.iter().find(|p| !p.chain.is_empty()) {
        let mut chain = param.name.clone();
        for link in &param.chain {
            if let Some(target_type) = &link.target_type {
                chain.push(':');
                chain.push_str(target_type);
            }
            chain.push('.');
            chain.push_str(&link.target_param);
        }
        return Err(StorageError::Search(
            SearchError::ChainedSearchNotSupported {
                chain: format!("{chain} (unresolved; resolve chains before search)"),
            },
        ));
    }
    if !query.reverse_chains.is_empty() {
        return Err(StorageError::Search(SearchError::ReverseChainNotSupported));
    }
    Ok(())
}

/// Refuses what `_contained` matching cannot apply. `:missing` was once the
/// only such criterion, hence the name; the full rule lives with the builder
/// it describes (#1363).
fn reject_contained_missing(query: &SearchQuery) -> StorageResult<()> {
    QueryBuilder::reject_unsupported_contained(query).map_err(StorageError::Search)
}

/// Old unfolded rows cannot prove the order of repeated component types.
/// Probe by parameter, before paging or count-zero shortcuts, using the
/// caller's connection so conditional transactions see their own index state.
fn legacy_composite_probe_sql(
    conn: &rusqlite::Connection,
    contained: bool,
) -> StorageResult<String> {
    use rusqlite::OptionalExtension;
    let (type_column, scope, index) = if contained {
        (
            "contained_type",
            "is_contained = 1",
            "idx_search_contained_composite_legacy",
        )
    } else {
        (
            "resource_type",
            "is_contained = 0",
            "idx_search_composite_legacy",
        )
    };
    // The broader contained index can win the equality-prefix heuristic and
    // visit every row after reindex. Prefer the sparse probe explicitly, but
    // preserve correct reads while bulk rebuild temporarily drops it.
    let exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
            [index],
            |_| Ok(()),
        )
        .optional()
        .or_query_error("Failed to read composite probe index metadata")?
        .is_some();
    let hint = if exists {
        format!(" INDEXED BY {index}")
    } else {
        String::new()
    };
    Ok(format!(
        "SELECT 1 FROM search_index{hint} WHERE tenant_id = ?1 AND {type_column} = ?2 \
                AND param_name = ?3 AND {scope} AND composite_group IS NOT NULL \
                AND composite_slot IS NULL LIMIT 1"
    ))
}

fn reject_legacy_composites(
    conn: &rusqlite::Connection,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> StorageResult<()> {
    use crate::types::{ContainedMode, SearchModifier, SearchParamType};

    for parameter in &query.parameters {
        if parameter.param_type != SearchParamType::Composite
            // Ordinary :missing tests presence only, without component order.
            || (query.contained == ContainedMode::Off
                && matches!(parameter.modifier, Some(SearchModifier::Missing)))
            || !parameter
                .components
                .iter()
                .enumerate()
                .any(|(position, component)| {
                    parameter.components[..position]
                        .iter()
                        .any(|earlier| earlier.param_type == component.param_type)
                })
        {
            continue;
        }
        for contained in [false, true] {
            if (contained && query.contained == ContainedMode::Off)
                || (!contained && query.contained == ContainedMode::On)
            {
                continue;
            }
            let sql = legacy_composite_probe_sql(conn, contained)?;
            let legacy = run_legacy_composite_probe(
                conn,
                &sql,
                tenant.tenant_id().as_str(),
                &query.resource_type,
                &parameter.name,
            )
            .or_query_error("Failed to probe legacy composite index rows")?;
            if legacy.is_some() {
                return Err(StorageError::Search(SearchError::InvalidComposite {
                    message: format!(
                        "search parameter '{}' requires $reindex: older SQLite composite index rows lack component slots; update all writers and reindex the full tenant",
                        parameter.name
                    ),
                }));
            }
        }
    }
    Ok(())
}

fn run_legacy_composite_probe(
    conn: &rusqlite::Connection,
    sql: &str,
    tenant: &str,
    resource_type: &str,
    parameter: &str,
) -> rusqlite::Result<Option<()>> {
    use rusqlite::OptionalExtension;
    let probe = |sql: &str| {
        conn.query_row(sql, params![tenant, resource_type, parameter], |_| Ok(()))
            .optional()
    };
    match probe(sql) {
        Err(error) => {
            // A bulk rebuild can drop the index after the metadata read. Retry
            // only that exact missing-index error, leaving every other failure
            // visible. A transaction that holds a schema snapshot needs no retry.
            let missing = match &error {
                rusqlite::Error::SqliteFailure(_, Some(message)) => {
                    message.strip_prefix("no such index: ").filter(|name| {
                        matches!(
                            *name,
                            "idx_search_composite_legacy" | "idx_search_contained_composite_legacy"
                        )
                    })
                }
                _ => None,
            };
            if let Some(index) = missing {
                let hint = format!(" INDEXED BY {index}");
                if sql.contains(&hint) {
                    return probe(&sql.replace(&hint, ""));
                }
            }
            Err(error)
        }
        result => result,
    }
}

/// A `_cursor` that decoded but carries a sort value of the wrong type for its
/// sort key. Cursors are unkeyed base64 JSON, so any client can craft one; this
/// is a bad request (400), not a server fault (#1120).
pub(super) fn invalid_cursor(cursor: &PageCursor) -> StorageError {
    StorageError::Search(SearchError::InvalidCursor {
        cursor: cursor.encode(),
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
                Some(CursorValue::Decimal(f)) => Some(*f),
                Some(CursorValue::Number(i)) => Some(*i as f64),
                Some(CursorValue::String(s)) => Some(s.parse().unwrap_or(0.0)),
                // A cursor on a resource with no value for the sort key.
                Some(CursorValue::Null) | None => None,
                _ => return Err(invalid_cursor(cursor)),
            };
            params.push(Box::new(n));
        }
        SortValueKind::Timestamp | SortValueKind::Text => match value {
            Some(CursorValue::String(s)) => params.push(Box::new(s.clone())),
            Some(CursorValue::Null) | None => params.push(Box::new(Option::<String>::None)),
            _ => return Err(invalid_cursor(cursor)),
        },
    }
    Ok(())
}

/// Whether a cursor sits on a resource with no value for the sort key.
fn cursor_value_is_null(cursor: &PageCursor) -> bool {
    matches!(cursor.sort_values().first(), Some(CursorValue::Null) | None)
}

/// The keyset `WHERE` condition and `ORDER BY` list for a cursor page, with
/// `?3` bound to the cursor's sort value and `?4` to its resource id.
///
/// A resource with no value for a nullable key sorts after every resource
/// that has one, in either direction (#1606). Paging forward from a present
/// value therefore still reaches the missing ones, and paging forward from a
/// missing one only walks the rest of them by id. `Previous` walks the exact
/// mirror: the reversed order puts missing values first, so from a present
/// value none of them qualify (a comparison with NULL is never true), and
/// from a missing one every present value lies before it.
fn keyset_page_clauses(k: &KeysetKey, cursor_is_null: bool, backward: bool) -> (String, String) {
    let e = &k.expr;
    let asc = k.direction == crate::types::SortDirection::Ascending;
    let at_missing = k.nullable && cursor_is_null;
    if backward {
        let condition = if at_missing {
            format!("({e} IS NOT NULL OR ({e} IS ?3 AND id < ?4))")
        } else {
            let op = if asc { "<" } else { ">" };
            format!("({e} {op} ?3 OR ({e} = ?3 AND id < ?4))")
        };
        let nulls = if k.nullable { " NULLS FIRST" } else { "" };
        let dir = if asc { "DESC" } else { "ASC" };
        (condition, format!("{e} {dir}{nulls}, id DESC"))
    } else {
        let condition = if at_missing {
            format!("({e} IS ?3 AND id > ?4)")
        } else {
            let op = if asc { ">" } else { "<" };
            let or_missing = if k.nullable {
                format!(" OR {e} IS NULL")
            } else {
                String::new()
            };
            format!("({e} {op} ?3 OR ({e} = ?3 AND id > ?4){or_missing})")
        };
        let nulls = if k.nullable { " NULLS LAST" } else { "" };
        let dir = if asc { "ASC" } else { "DESC" };
        (condition, format!("{e} {dir}{nulls}, id ASC"))
    }
}

impl SqliteBackend {
    /// The body of [`SearchProvider::search`], run on a caller-supplied
    /// connection.
    ///
    /// `SearchProvider::search` takes a fresh pooled connection, which under
    /// `BEGIN IMMEDIATE` sees pre-transaction state only. A bundle entry that
    /// must resolve `ifNoneExist` against what earlier entries in the same
    /// transaction wrote runs this on the transaction's own connection instead
    /// (via `SqliteTransaction::with_connection`, #511). `total` is computed by
    /// the caller because `search_count` is async and this is not.
    #[allow(clippy::type_complexity)]
    pub(crate) fn search_with_connection(
        &self,
        conn: &rusqlite::Connection,
        tenant: &TenantContext,
        query: &SearchQuery,
        total: Option<u64>,
    ) -> StorageResult<SearchResult> {
        self.search_with_connection_probe(conn, tenant, query, total, probe_enabled())
    }

    /// [`Self::search_with_connection`] with the bounded probe forced on or
    /// off (`HFS_SQLITE_SEARCH_PROBE` decides in production).
    #[allow(clippy::type_complexity)]
    pub(crate) fn search_with_connection_probe(
        &self,
        conn: &rusqlite::Connection,
        tenant: &TenantContext,
        query: &SearchQuery,
        total: Option<u64>,
        probe: bool,
    ) -> StorageResult<SearchResult> {
        with_composite_rewrite(conn, query, |rewrite| {
            self.search_with_connection_plan(conn, tenant, query, total, probe, rewrite)
        })
    }

    /// [`Self::search_with_connection_probe`] with the code-first composite
    /// pairing forced on or off.
    #[allow(clippy::type_complexity)]
    pub(crate) fn search_with_connection_plan(
        &self,
        conn: &rusqlite::Connection,
        tenant: &TenantContext,
        query: &SearchQuery,
        total: Option<u64>,
        probe: bool,
        composite_rewrite: bool,
    ) -> StorageResult<SearchResult> {
        reject_unsupported_metadata_modifier(query)?;
        reject_legacy_composites(conn, tenant, query)?;

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

        let search_filter = if !query.parameters.is_empty() || query.compartment.is_some() {
            let builder = QueryBuilder::new(tenant_id, resource_type)
                .with_param_offset(param_offset)
                .with_composite_rewrite(composite_rewrite);
            let fragment = builder.build(query);
            if !fragment.sql.is_empty() {
                Some(fragment)
            } else {
                None
            }
        } else {
            None
        };
        let mut filter_clause = search_filter
            .as_ref()
            .map(|f| format!(" AND rowid IN ({})", f.sql))
            .unwrap_or_default();
        let first_page_source = first_page_source(search_filter.is_some(), query, count);
        // First page of a one-parameter membership search: the probe may pick
        // a cheaper plan for it (same rows, same order).
        let mut probed_keys: Option<Vec<i64>> = None;
        let first_page_plan = match &search_filter {
            Some(f)
                if probe
                    && cursor.is_none()
                    && query.offset.is_none()
                    && query.sort.is_empty()
                    && query.compartment.is_none()
                    && query.parameters.len() == 1
                    && count > 1 =>
            {
                let (plan, keys) =
                    probe_first_page(conn, tenant_id, resource_type, &f.sql, &f.params);
                if plan == FirstPagePlan::ListDriven {
                    probed_keys = Some(keys);
                }
                plan
            }
            _ => FirstPagePlan::Planner,
        };
        // A short key list is already in hand: the page reads those rows by
        // rowid instead of evaluating the filter a second time (a pairing of
        // token and quantity rows costs as much as the probe did).
        if let Some(keys) = &probed_keys {
            let placeholders = (0..keys.len())
                .map(|i| format!("?{}", param_offset + 1 + i))
                .collect::<Vec<_>>()
                .join(", ");
            filter_clause = format!(" AND rowid IN ({placeholders})");
        }

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

        // The bounded early-exit attempts, tried before the page statement and
        // kept only when one fills the page: the newest [`WINDOWS`] resources
        // in turn. A short window (the filter matches few of the newest
        // resources) widens before the page statement runs, since a miss only
        // costs the window.
        let window_sqls: Vec<String> = match (first_page_plan, &search_filter) {
            (FirstPagePlan::ExistsDriven, Some(f)) => membership_exists_predicate(&f.sql, "w.rk")
                .map(|exists| {
                    WINDOWS
                        .iter()
                        .map(|window| {
                            format!(
                                "SELECT {select_cols} FROM \
                                 (SELECT rowid AS rk, last_updated AS lu, id AS rid \
                                  FROM resources INDEXED BY idx_resources_reindex \
                                  WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 \
                                  ORDER BY last_updated DESC, id ASC LIMIT {window}) AS w \
                                 CROSS JOIN resources ON resources.rowid = w.rk \
                                 WHERE {exists} \
                                 ORDER BY w.lu DESC, w.rid ASC LIMIT {lim}",
                                lim = count + 1,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let search_params = match probed_keys {
            Some(keys) => keys.into_iter().map(SqlParam::Integer).collect(),
            None => search_filter.map(|f| f.params).unwrap_or_default(),
        };

        // Build query based on pagination mode.
        let (sql, has_previous) = if let (Some(cursor), Some(k)) = (&cursor, &keyset) {
            let backward = cursor.direction() == CursorDirection::Previous;
            let (condition, order) = keyset_page_clauses(k, cursor_value_is_null(cursor), backward);
            let sql = format!(
                "SELECT {cols} FROM resources \
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0{filter} \
                 AND {condition} \
                 ORDER BY {order} LIMIT {lim}",
                cols = select_cols,
                filter = filter_clause,
                lim = count + 1,
            );
            // Backward: the placeholder is replaced by the extra-row check below.
            (sql, !backward)
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
            let page = |source: &str, filter: &str| {
                format!(
                    "SELECT {cols} FROM {source} \
                     WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0{filter} \
                     {order} LIMIT {lim}",
                    cols = select_cols,
                    order = order_by,
                    lim = count + 1,
                )
            };
            let sql = match first_page_plan {
                FirstPagePlan::ListDriven => page("resources NOT INDEXED", &filter_clause),
                _ => page(first_page_source, &filter_clause),
            };
            (sql, false)
        };

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
        let run = |sql: &str| -> StorageResult<Vec<RawRow>> {
            let mut stmt = conn
                .prepare(sql)
                .or_query_error("Failed to prepare search query")?;
            stmt.query_map(param_refs.as_slice(), |row| {
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
            .or_query_error("Failed to execute search")?
            .collect::<Result<Vec<_>, _>>()
            .or_query_error("Failed to read row")
        };
        // A window attempt may fail to prepare (an index dropped for a bulk
        // load) or fall short of a full page: either way the next, wider
        // window and finally the page statement run.
        let windowed = window_sqls
            .iter()
            .find_map(|window| run(window).ok().filter(|rows| rows.len() > count));
        let raw_rows = match windowed {
            Some(rows) => rows,
            None => run(&sql)?,
        };

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

        let backward = cursor
            .as_ref()
            .is_some_and(|c| c.direction() == CursorDirection::Previous);

        // Forward: the extra row (if any) is the *last* one and proves a next page.
        // Backward: rows arrive in reversed order, so the extra row is also the
        // last one fetched but it is the *farthest* from the cursor — it belongs to
        // page N-2, not to the page we return — and it proves a previous page. It
        // must be dropped before `reverse()` restores the sort order. See #1079
        // and the Elasticsearch `backward` branch (#1015).
        let (has_next, has_previous) = if backward {
            let has_previous = parsed.len() > count;
            if has_previous {
                parsed.pop();
            }
            parsed.reverse();
            (!parsed.is_empty(), has_previous)
        } else {
            let has_next = parsed.len() > count;
            if has_next {
                parsed.pop();
            }
            (has_next, has_previous)
        };

        // A sort with no keyset (multi-field) pages by offset, which is what
        // the REST layer's `next` link falls back to without a cursor. A
        // cursor minted here would be ignored on the way back in and return
        // the first page again (#1606, as MongoDB since #1058).
        let next_cursor = if has_next && keyset.is_some() {
            parsed.last().map(|(r, sk)| {
                PageCursor::new(vec![sk.clone().unwrap_or(CursorValue::Null)], r.id()).encode()
            })
        } else {
            None
        };
        let previous_cursor = if has_previous && keyset.is_some() {
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
            scores: Default::default(),
        })
    }
}

/// One `resources` row as the page statements return it, with its sort key.
type RawRow = (String, String, Vec<u8>, String, String, Option<CursorValue>);

/// Newest `resources` rows the early-exit attempts look at, narrowest first.
/// A page the 512 newest cannot fill (a date range that the newest resources
/// mostly miss) is retried on 4,096 before the planner's plan runs.
const WINDOWS: [usize; 2] = [512, 4096];

/// Rows of a single parameter's index the probe reads at most. A filter with
/// fewer rows is "selective": its whole key list is cheap to build.
const PROBE_CAP: usize = 2001;

/// Whether the bounded probe is on: `HFS_SQLITE_SEARCH_PROBE`, read once.
/// Unset or empty means on; `0`, `false`, `off` or `no` turn it off.
fn probe_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| match std::env::var("HFS_SQLITE_SEARCH_PROBE") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off" | "no"
        ),
        Err(_) => true,
    })
}

/// How the first page of a one-parameter membership search reaches its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FirstPagePlan {
    /// The planner's choice: `rowid IN (<keys>)` with `resources` walked in
    /// `last_updated` order.
    Planner,
    /// The key list is short (under [`PROBE_CAP`] index rows): drive from it
    /// and read `resources` by rowid, sorting the few rows. The probe's own
    /// keys are bound to the page statement, so the filter runs once.
    ListDriven,
    /// The key list is long: try the newest [`WINDOWS`] resources in page
    /// order, each tested against the index with a correlated `EXISTS`; the
    /// page is final when the window fills it, else the planner's plan runs.
    ExistsDriven,
}

/// Picks the [`FirstPagePlan`] with one bounded probe over the filter's own
/// index rows (`SELECT … FROM (<rows> LIMIT 2001)`, a seek that stops early).
/// Any shape other than one plain parameter membership, and any probe error,
/// is [`FirstPagePlan::Planner`]: the result of every plan is the same set in
/// the same `ORDER BY last_updated DESC, id` order, so this only chooses cost.
///
/// Only the tests call it; the production path reaches the same plans through
/// the probe's own keys.
#[cfg(test)]
fn choose_first_page_plan(
    conn: &rusqlite::Connection,
    tenant_id: &str,
    resource_type: &str,
    rows_sql: &str,
    filter_params: &[SqlParam],
) -> FirstPagePlan {
    probe_first_page(conn, tenant_id, resource_type, rows_sql, filter_params).0
}

/// [`choose_first_page_plan`], and for [`FirstPagePlan::ListDriven`] the
/// distinct `resource_key`s the probe read (under [`PROBE_CAP`] of them), so
/// the page can be read by rowid without evaluating the filter again.
fn probe_first_page(
    conn: &rusqlite::Connection,
    tenant_id: &str,
    resource_type: &str,
    rows_sql: &str,
    filter_params: &[SqlParam],
) -> (FirstPagePlan, Vec<i64>) {
    if !is_parameter_rows(rows_sql) {
        return (FirstPagePlan::Planner, Vec::new());
    }
    let probe = format!("SELECT resource_key FROM ({rows_sql} LIMIT {PROBE_CAP})");
    let mut bound: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(tenant_id.to_string()),
        Box::new(resource_type.to_string()),
    ];
    for param in filter_params {
        bound.push(match param {
            SqlParam::String(s) => Box::new(s.clone()),
            SqlParam::Integer(i) => Box::new(*i),
            SqlParam::Float(f) => Box::new(*f),
            SqlParam::Null => Box::new(Option::<String>::None),
        });
    }
    let refs: Vec<&dyn rusqlite::ToSql> = bound.iter().map(|p| p.as_ref()).collect();
    let keys = conn.prepare(&probe).and_then(|mut stmt| {
        stmt.query_map(refs.as_slice(), |row| row.get::<_, Option<i64>>(0))?
            .collect::<Result<Vec<_>, _>>()
    });
    match keys {
        Ok(keys) if keys.len() < PROBE_CAP => {
            let mut keys: Vec<i64> = keys.into_iter().flatten().collect();
            keys.sort_unstable();
            keys.dedup();
            (FirstPagePlan::ListDriven, keys)
        }
        Ok(_) => (FirstPagePlan::ExistsDriven, Vec::new()),
        Err(_) => (FirstPagePlan::Planner, Vec::new()),
    }
}

/// The table expression of a first page (no cursor, no offset) of `count`
/// resources: `resources`, or `resources NOT INDEXED` for a filtered one-row
/// page on the default order.
///
/// SQLite plans that page (`LIMIT` 1 or 2) by reading every key of the
/// filter's `rowid IN (…)` list and sorting them: it probes
/// `idx_resources_live_type` for each key, then reads the row anyway for its
/// `data`, so a broad filter (`date=le…`, `ne…`) paid two lookups per match
/// (#1506). `NOT INDEXED` keeps that plan with the row read alone. From three
/// rows up, and for `_sort=_lastUpdated` or `_id`, SQLite walks an index in
/// the page order and stops when the page is full, which `NOT INDEXED` would
/// forbid, so those pages and every `_sort` are left to the planner.
fn first_page_source(filtered: bool, query: &SearchQuery, count: usize) -> &'static str {
    if filtered && query.sort.is_empty() && count <= 1 {
        "resources NOT INDEXED"
    } else {
        "resources"
    }
}

/// Whether `query` can use the code-first composite pairing: it has a
/// composite parameter and both indexes the pairing is hinted to exist, the
/// token index in its code-first shape (a database still carrying the earlier,
/// system-first `idx_search_token` would answer the hinted statement with a
/// table lookup per token row). `INDEXED BY` makes a statement fail to prepare
/// against a missing index, and a bulk rebuild drops the value indexes for its
/// duration.
fn composite_rewrite_available(conn: &rusqlite::Connection, query: &SearchQuery) -> bool {
    use crate::types::SearchParamType;
    if !query
        .parameters
        .iter()
        .any(|p| p.param_type == SearchParamType::Composite)
    {
        return false;
    }
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND ( \
            name = 'idx_search_composite_cov' \
            OR (name = 'idx_search_token' AND sql LIKE '%value_token_code, resource_key, value_token_system, composite_group%'))",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n == 2)
    .unwrap_or(false)
}

/// Runs `run(rewrite)` with the composite pairing on when
/// [`composite_rewrite_available`], and again with it off if an index was
/// dropped between that check and the statement's prepare.
fn with_composite_rewrite<T>(
    conn: &rusqlite::Connection,
    query: &SearchQuery,
    run: impl Fn(bool) -> StorageResult<T>,
) -> StorageResult<T> {
    if !composite_rewrite_available(conn, query) {
        return run(false);
    }
    match run(true) {
        Err(error) if error.to_string().contains("no such index") => run(false),
        result => result,
    }
}

/// The id-only page of [`SearchProvider::search_ids`], run on a blocking
/// thread by the caller.
fn search_ids_on(
    conn: &rusqlite::Connection,
    tenant: &TenantContext,
    query: &SearchQuery,
    cursor: Option<PageCursor>,
) -> StorageResult<Page<String>> {
    with_composite_rewrite(conn, query, |rewrite| {
        search_ids_on_plan(conn, tenant, query, cursor.clone(), rewrite)
    })
}

fn search_ids_on_plan(
    conn: &rusqlite::Connection,
    tenant: &TenantContext,
    query: &SearchQuery,
    cursor: Option<PageCursor>,
    composite_rewrite: bool,
) -> StorageResult<Page<String>> {
    let tenant_id = tenant.tenant_id().as_str();
    let resource_type = &query.resource_type;
    let param_offset = if cursor.is_some() { 4 } else { 2 };
    let search_filter = if !query.parameters.is_empty() {
        let fragment = QueryBuilder::new(tenant_id, resource_type)
            .with_param_offset(param_offset)
            .with_composite_rewrite(composite_rewrite)
            .build(query);
        if fragment.sql.is_empty() {
            None
        } else {
            Some(fragment)
        }
    } else {
        None
    };
    let filter_clause = search_filter
        .as_ref()
        .map(|fragment| format!(" AND rowid IN ({})", fragment.sql))
        .unwrap_or_default();
    let search_params = search_filter
        .map(|fragment| fragment.params)
        .unwrap_or_default();
    let cursor_clause = if cursor.is_some() {
        " AND (last_updated < ?3 OR (last_updated = ?3 AND id > ?4))"
    } else {
        ""
    };
    let count = query.count.unwrap_or(100) as usize;
    let sql = format!(
        "SELECT id, last_updated FROM resources \
             WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0{filter_clause}{cursor_clause} \
             ORDER BY last_updated DESC, id ASC LIMIT {}",
        count + 1
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(tenant_id.to_string()),
        Box::new(resource_type.to_string()),
    ];
    if let Some(cursor) = &cursor {
        bind_cursor_value(&mut params, SortValueKind::Timestamp, cursor)?;
        params.push(Box::new(cursor.resource_id().to_string()));
    }
    for param in &search_params {
        match param {
            SqlParam::String(value) => params.push(Box::new(value.clone())),
            SqlParam::Integer(value) => params.push(Box::new(*value)),
            SqlParam::Float(value) => params.push(Box::new(*value)),
            SqlParam::Null => params.push(Box::new(Option::<String>::None)),
        }
    }
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|param| param.as_ref()).collect();
    reject_legacy_composites(conn, tenant, query)?;
    let mut stmt = conn
        .prepare(&sql)
        .or_query_error("Failed to prepare id-only search query")?;
    let mut rows: Vec<(String, String)> = stmt
        .query_map(param_refs.as_slice(), |row| Ok((row.get(0)?, row.get(1)?)))
        .or_query_error("Failed to execute id-only search")?
        .collect::<Result<Vec<_>, _>>()
        .or_query_error("Failed to read id-only search row")?;
    let has_next = rows.len() > count;
    if has_next {
        rows.pop();
    }
    let has_previous = cursor.is_some();
    let next_cursor = if has_next {
        rows.last().map(|(id, updated)| {
            PageCursor::new(vec![CursorValue::String(updated.clone())], id).encode()
        })
    } else {
        None
    };
    let previous_cursor = if has_previous {
        rows.first().map(|(id, updated)| {
            PageCursor::previous(vec![CursorValue::String(updated.clone())], id).encode()
        })
    } else {
        None
    };
    Ok(Page::new(
        rows.into_iter().map(|(id, _)| id).collect(),
        PageInfo {
            next_cursor,
            previous_cursor,
            total: None,
            has_next,
            has_previous,
        },
    ))
}

/// The count of [`SearchProvider::search_count`] for a query without
/// `_contained`, run on a blocking thread by the caller.
/// The distinct indexed values of `reference_param` on the current,
/// top-level resources matching `query`, for
/// [`SearchProvider::search_reference_values`]. One statement: the filter's
/// candidates are evaluated once, not once per page (#1930).
fn search_reference_values_on(
    conn: &rusqlite::Connection,
    tenant: &TenantContext,
    query: &SearchQuery,
    reference_param: &str,
) -> StorageResult<Vec<String>> {
    reject_legacy_composites(conn, tenant, query)?;
    let (sql, params) = reference_values_statement(tenant, query, reference_param);
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn
        .prepare(&sql)
        .or_query_error("Failed to prepare reference-value query")?;
    let values = stmt
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
        .or_query_error("Failed to execute reference-value query")?
        .collect::<Result<Vec<_>, _>>()
        .or_query_error("Failed to read reference-value row")?;
    Ok(values)
}

/// The statement of [`search_reference_values_on`] and its bound values.
fn reference_values_statement(
    tenant: &TenantContext,
    query: &SearchQuery,
    reference_param: &str,
) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let tenant_id = tenant.tenant_id().as_str();
    let resource_type = &query.resource_type;

    let fragment = (!query.parameters.is_empty()).then(|| {
        QueryBuilder::new(tenant_id, resource_type)
            .with_param_offset(2)
            .build(query)
    });
    let filter = fragment
        .as_ref()
        .filter(|fragment| !fragment.sql.is_empty())
        .map(|fragment| format!(" AND r.rowid IN ({})", fragment.sql))
        .unwrap_or_default();
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(tenant_id.to_string()),
        Box::new(resource_type.to_string()),
    ];
    for param in fragment.iter().flat_map(|fragment| &fragment.params) {
        match param {
            SqlParam::String(value) => params.push(Box::new(value.clone())),
            SqlParam::Integer(value) => params.push(Box::new(*value)),
            SqlParam::Float(value) => params.push(Box::new(*value)),
            SqlParam::Null => params.push(Box::new(Option::<String>::None)),
        }
    }
    params.push(Box::new(reference_param.to_string()));
    let param_name = params.len();

    // Drive from the candidates and probe `search_index` per candidate:
    // `CROSS JOIN` keeps the candidates as the outer loop, the
    // `(tenant_id, resource_type)` prefix lets the `resource_key` probe seek
    // `idx_search_composite` (#1197), and the unary `+` keeps the planner off
    // `idx_search_reference`, which would read every row of the parameter.
    let sql = format!(
        "SELECT DISTINCT si.value_reference \
         FROM (SELECT r.rowid AS resource_key FROM resources r \
                WHERE r.tenant_id = ?1 AND r.resource_type = ?2 AND r.is_deleted = 0{filter}) c \
         CROSS JOIN search_index si \
         WHERE si.tenant_id = ?1 AND si.resource_type = ?2 \
           AND si.resource_key = c.resource_key \
           AND +si.param_name = ?{param_name} \
           AND si.is_contained = 0 \
           AND si.value_reference IS NOT NULL"
    );
    (sql, params)
}

fn search_count_on(
    conn: &rusqlite::Connection,
    tenant: &TenantContext,
    query: &SearchQuery,
) -> StorageResult<u64> {
    with_composite_rewrite(conn, query, |rewrite| {
        search_count_on_plan(conn, tenant, query, rewrite)
    })
}

fn search_count_on_plan(
    conn: &rusqlite::Connection,
    tenant: &TenantContext,
    query: &SearchQuery,
    composite_rewrite: bool,
) -> StorageResult<u64> {
    reject_legacy_composites(conn, tenant, query)?;
    let tenant_id = tenant.tenant_id().as_str();
    let resource_type = &query.resource_type;

    // Build the search filter if there are search parameters or a compartment.
    let (sql, all_params): (String, Vec<Box<dyn rusqlite::ToSql>>) = if !query.parameters.is_empty()
        || query.compartment.is_some()
    {
        let builder = QueryBuilder::new(tenant_id, resource_type)
            .with_param_offset(2)
            .with_composite_rewrite(composite_rewrite);
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
            "SELECT COUNT(*) FROM resources WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 AND rowid IN ({})",
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
        .or_query_error("Failed to count resources")?;

    Ok(count as u64)
}

#[async_trait]
impl SearchProvider for SqliteBackend {
    async fn search(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        reject_contained_missing(query)?;
        reject_unsupported_metadata_modifier(query)?;

        // `_contained` search uses a dedicated path (different index columns and
        // heterogeneous result types); standard search handles `_contained=false`.
        // This is the only entry point into that path, so the gate above is not
        // repeated inside `search_contained` itself.
        if query.contained != crate::types::ContainedMode::Off {
            return self.search_contained(tenant, query).await;
        }

        // Populate Bundle.total only when the client asked for it
        // (`_total=accurate|estimate`). Computed up-front, before acquiring the
        // (non-Send) connection, so it is not held across this await.
        let total = if query.wants_total() {
            Some(self.search_count(tenant, query).await?)
        } else {
            None
        };

        // On a blocking thread, so that a chain resolver draining many pages
        // yields between them: the request timeout can then fire and a
        // dropped request stops after the page in flight (#1930).
        let this = self.clone();
        let tenant = tenant.clone();
        let query = query.clone();
        self.run_blocking(move |conn| this.search_with_connection(conn, &tenant, &query, total))
            .await
    }

    async fn search_ids(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<Page<String>> {
        // The resolver issues plain, unsorted single-type searches. Preserve
        // the full search path for other query shapes rather than duplicating
        // their result shaping here.
        let cursor = query
            .cursor
            .as_ref()
            .and_then(|value| PageCursor::decode(value).ok());
        if !query.sort.is_empty()
            || query.offset.is_some()
            || query.contained != crate::types::ContainedMode::Off
            || !query.includes.is_empty()
            || query.total.is_some()
            || query.summary.is_some()
            || !query.elements.is_empty()
            || query.compartment.is_some()
            || !query.list.is_empty()
            || !query.reverse_chains.is_empty()
            || (query.cursor.is_some() && cursor.is_none())
            || cursor
                .as_ref()
                .is_some_and(|value| value.direction() != CursorDirection::Next)
        {
            return Ok(self
                .search(tenant, query)
                .await?
                .resources
                .map(|resource| resource.id().to_string()));
        }

        reject_contained_missing(query)?;
        reject_unsupported_metadata_modifier(query)?;
        let tenant = tenant.clone();
        let query = query.clone();
        self.run_blocking(move |conn| search_ids_on(conn, &tenant, &query, cursor))
            .await
    }

    async fn search_count(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<u64> {
        reject_contained_missing(query)?;
        reject_unsupported_metadata_modifier(query)?;

        // Under `_contained` the count is of what `search` returns (#1383),
        // not of the top-level resources matching the same criteria.
        if query.contained != crate::types::ContainedMode::Off {
            let plan = self.contained_plan(tenant, query).await?;
            return Ok(plan.top_total + plan.keys.len() as u64);
        }

        let tenant = tenant.clone();
        let query = query.clone();
        self.run_blocking(move |conn| search_count_on(conn, &tenant, &query))
            .await
    }

    async fn search_reference_values(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
        reference_param: &str,
    ) -> StorageResult<Option<Vec<String>>> {
        // Only the plain single-type filter the resolver builds, over an index
        // this backend maintains; anything else keeps the paged path.
        if self.is_search_offloaded()
            || !query.sort.is_empty()
            || query.offset.is_some()
            || query.cursor.is_some()
            || query.contained != crate::types::ContainedMode::Off
            || !query.includes.is_empty()
            || query.total.is_some()
            || query.summary.is_some()
            || !query.elements.is_empty()
            || query.compartment.is_some()
            || !query.list.is_empty()
            || !query.reverse_chains.is_empty()
        {
            return Ok(None);
        }
        reject_contained_missing(query)?;
        reject_unsupported_metadata_modifier(query)?;
        let tenant = tenant.clone();
        let query = query.clone();
        let reference_param = reference_param.to_string();
        self.run_blocking(move |conn| {
            search_reference_values_on(conn, &tenant, &query, &reference_param)
        })
        .await
        .map(Some)
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
            .or_query_error("Failed to prepare multi-type search")?;

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
            .or_query_error("Failed to execute multi-type search")?;

        let mut resources = Vec::new();
        for row in rows {
            let (resource_type, id, version_id, data, last_updated_str, fhir_version_str) =
                row.or_query_error("Failed to read row")?;

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
            scores: Default::default(),
        })
    }
}

#[async_trait]
impl IncludeProvider for SqliteBackend {
    /// Delegates to the shared, registry-driven resolver so `_include` (and
    /// `:iterate`) follows the same search-parameter definitions (with FHIRPath
    /// expression evaluation) used elsewhere; SQLite does not resolve includes
    /// inline in `search()`.
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

            let mut stmt = conn
                .prepare(&sql)
                .or_query_error("Failed to prepare revinclude query")?;

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
                .or_query_error("Failed to execute revinclude query")?;

            for row in rows {
                let (id, version_id, data, last_updated_str, fhir_version_str) =
                    row.or_query_error("Failed to read row")?;

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
impl SqliteBackend {
    /// Executes a `_contained=true|both` search.
    ///
    /// Matches contained resources of `query.resource_type` via the
    /// `is_contained` index rows, then returns either the container resources
    /// (`_containedType=container`, default) or the contained resources
    /// themselves (`_containedType=contained`). For `_contained=both`, top-level
    /// matches (run through the standard path) are merged in first.
    ///
    /// Paginated by `_offset`/`_count` (no keyset cursor) over the result list
    /// [`Self::contained_plan`] describes — top-level matches, then contained
    /// ones — so `_total`, `search_count` and every page agree (#1383). Only
    /// the requested window is materialized.
    async fn search_contained(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<SearchResult> {
        use crate::types::ContainedReturn;

        let contained_type = query.resource_type.as_str();
        let count = query.count.unwrap_or(100) as usize;
        let offset = query.offset.unwrap_or(0) as u64;
        let plan = self.contained_plan(tenant, query).await?;

        // Top-level matches (`_contained=both`) come first.
        let mut items: Vec<StoredResource> = Vec::new();
        if let Some(top_query) = &plan.top_query {
            if offset < plan.top_total {
                let mut page_query = top_query.clone();
                page_query.offset = Some(offset as u32);
                page_query.count = Some(count as u32);
                items = self.search(tenant, &page_query).await?.resources.items;
                items.truncate(count);
            }
        }

        let skip = offset.saturating_sub(plan.top_total) as usize;
        let room = count.saturating_sub(items.len());
        for (ctype, cid, local) in plan.keys.iter().skip(skip).take(room) {
            let Some(container) = self.read(tenant, ctype, cid).await? else {
                continue;
            };
            match (query.contained_return, local) {
                (ContainedReturn::Contained, Some(local_id)) => {
                    if let Some(c) = extract_contained_resource(container.content(), local_id) {
                        items.push(build_contained_stored(
                            &container,
                            contained_type,
                            local_id,
                            c,
                        ));
                    }
                }
                _ => items.push(container),
            }
        }

        let mut result = SearchResult::new(Page::new(items, PageInfo::end()));
        if query.wants_total() {
            result = result.with_total(plan.top_total + plan.keys.len() as u64);
        }
        Ok(result)
    }

    /// The result list of a `_contained=true|both` search, unmaterialized:
    /// `top_total` top-level matches (`both` only, served by `top_query`)
    /// followed by `keys`, one per contained match — a container
    /// (`_containedType=container`, local id `None`) or a contained resource —
    /// in a stable order. A container that is itself a top-level match is
    /// listed once, in the top-level part. A contained resource never is one:
    /// its local id may equal a top-level resource's id without being it.
    async fn contained_plan(
        &self,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> StorageResult<ContainedPlan> {
        use crate::types::{ContainedMode, ContainedReturn, SearchParamType, SearchParameter};

        {
            let conn = self.get_connection()?;
            reject_legacy_composites(&conn, tenant, query)?;
        }
        let mut keys = self.contained_matches(tenant.tenant_id().as_str(), query)?;
        if query.contained_return == ContainedReturn::Container {
            for key in &mut keys {
                key.2 = None;
            }
        } else {
            keys.retain(|key| key.2.is_some());
        }
        // A stable order to page over, one key per result.
        keys.sort();
        keys.dedup();

        if query.contained != ContainedMode::Both {
            return Ok(ContainedPlan {
                top_query: None,
                top_total: 0,
                keys,
            });
        }

        let mut top_query = query.clone();
        top_query.contained = ContainedMode::Off;
        top_query.contained_return = ContainedReturn::Container;
        let top_total = self.search_count(tenant, &top_query).await?;

        if query.contained_return == ContainedReturn::Container {
            let same_type: Vec<&str> = keys
                .iter()
                .filter(|key| key.0 == query.resource_type)
                .map(|key| key.1.as_str())
                .collect();
            let mut also_top_level: HashSet<String> = HashSet::new();
            for chunk in same_type.chunks(500) {
                let mut among = top_query.clone();
                among.parameters.push(SearchParameter {
                    name: "_id".to_string(),
                    param_type: SearchParamType::Token,
                    values: chunk.iter().map(|id| SearchValue::eq(*id)).collect(),
                    ..Default::default()
                });
                among.count = Some(chunk.len() as u32);
                among.offset = None;
                among.cursor = None;
                among.total = None;
                among.includes.clear();
                let found = self.search(tenant, &among).await?;
                also_top_level.extend(found.resources.items.iter().map(|r| r.id().to_string()));
            }
            keys.retain(|key| !(key.0 == query.resource_type && also_top_level.contains(&key.1)));
        }

        Ok(ContainedPlan {
            top_query: Some(top_query),
            top_total,
            keys,
        })
    }

    /// Resolves the contained matches of `query` →
    /// `(container_type, container_id, local_id)`, unordered.
    /// Synchronous: the connection is not `Send` and must not live across an
    /// await.
    fn contained_matches(
        &self,
        tenant_id: &str,
        query: &SearchQuery,
    ) -> StorageResult<Vec<(String, String, Option<String>)>> {
        let contained_type = query.resource_type.as_str();
        let builder = QueryBuilder::new(tenant_id, contained_type);
        let matches: Vec<(String, String, Option<String>)> = match builder.build_contained(query) {
            Some(fragment) => {
                let conn = self.get_connection()?;
                let mut stmt = conn
                    .prepare(&fragment.sql)
                    .or_query_error("Failed to prepare contained query")?;
                let mut all_params: Vec<Box<dyn rusqlite::ToSql>> = vec![
                    Box::new(tenant_id.to_string()),
                    Box::new(contained_type.to_string()),
                ];
                for param in &fragment.params {
                    match param {
                        SqlParam::String(s) => all_params.push(Box::new(s.clone())),
                        SqlParam::Integer(i) => all_params.push(Box::new(*i)),
                        SqlParam::Float(f) => all_params.push(Box::new(*f)),
                        SqlParam::Null => all_params.push(Box::new(Option::<String>::None)),
                    }
                }
                let refs: Vec<&dyn rusqlite::ToSql> =
                    all_params.iter().map(|p| p.as_ref()).collect();
                stmt.query_map(refs.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .or_query_error("Failed to execute contained query")?
                .collect::<Result<Vec<_>, _>>()
                .or_query_error("Failed to read contained row")?
            }
            None => Vec::new(),
        };
        Ok(matches)
    }
}

/// See [`SqliteBackend::contained_plan`].
struct ContainedPlan {
    top_query: Option<SearchQuery>,
    top_total: u64,
    keys: Vec<(String, String, Option<String>)>,
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
            .or_query_error("Failed to prepare find query")?;

        let rows = stmt
            .query_map(params![tenant_id, resource_type, param_name], |row| {
                row.get::<_, String>(0)
            })
            .or_query_error("Failed to execute find query")?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.or_query_error("Failed to read row")?);
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
            .or_query_error("Failed to prepare query")?;

        let rows = stmt
            .query_map(params![tenant_id, resource_type], |row| {
                let id: String = row.get(0)?;
                let version_id: String = row.get(1)?;
                let data: Vec<u8> = row.get(2)?;
                let last_updated: String = row.get(3)?;
                let fhir_version: String = row.get(4)?;
                Ok((id, version_id, data, last_updated, fhir_version))
            })
            .or_query_error("Failed to query resources")?;

        let mut resources = Vec::new();
        for row in rows {
            let (id, version_id, data, last_updated_str, fhir_version_str) =
                row.or_query_error("Failed to read row")?;

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

    #[test]
    fn a_type_mismatched_cursor_sort_value_is_a_client_error() {
        // Crafted: a boolean where the Timestamp sort key expects an RFC3339
        // string. Must surface as a 400 InvalidCursor, not a 500 (#1120).
        let cursor = PageCursor::new(vec![CursorValue::Boolean(true)], "p1");
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let err = bind_cursor_value(&mut params, SortValueKind::Timestamp, &cursor)
            .expect_err("a boolean is not a timestamp");
        match err {
            StorageError::Search(SearchError::InvalidCursor { cursor: c }) => {
                assert_eq!(c, cursor.encode())
            }
            other => panic!("expected InvalidCursor, got {other:?}"),
        }

        // A well-typed cursor still binds.
        let ok = PageCursor::new(
            vec![CursorValue::String("2024-01-01T00:00:00Z".to_string())],
            "p1",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        assert!(bind_cursor_value(&mut params, SortValueKind::Timestamp, &ok).is_ok());
    }

    fn create_test_backend() -> SqliteBackend {
        // Point at the workspace's data directory so the search-parameter
        // registry loads the full FHIR spec (otherwise only the handful of
        // minimal embedded params are available and chained-search tests fail to
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
    async fn composite_legacy_guard_uses_the_supplied_transaction_connection() {
        use crate::types::{CompositeSearchComponent, SearchParamType, SearchValue};
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        backend.create(&tenant,"Observation",json!({"resourceType":"Observation","id":"o","status":"final","code":{"coding":[{"code":"A"}]},"valueCodeableConcept":{"coding":[{"code":"B"}]}}),FhirVersion::default()).await.unwrap();
        let mut query = SearchQuery::new("Observation");
        query.count = Some(0);
        query.parameters.push(SearchParameter {
            name: "code-value-concept".into(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq("missing$missing")],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".into(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "value-concept".into(),
                },
            ],
            ..Default::default()
        });
        let explain_real_probe = |conn: &rusqlite::Connection| {
            for contained in [false, true] {
                let sql = format!(
                    "EXPLAIN QUERY PLAN {}",
                    legacy_composite_probe_sql(conn, contained).unwrap()
                );
                let plan = conn
                    .prepare(&sql)
                    .unwrap()
                    .query_map(
                        params![
                            tenant.tenant_id().as_str(),
                            "Observation",
                            "code-value-concept"
                        ],
                        |row| row.get::<_, String>(3),
                    )
                    .unwrap()
                    .map(Result::unwrap)
                    .collect::<Vec<_>>()
                    .join(" | ");
                let index = if contained {
                    "idx_search_contained_composite_legacy"
                } else {
                    "idx_search_composite_legacy"
                };
                assert!(plan.contains(index), "real probe {plan}");
                assert!(!plan.contains("SCAN search_index"), "{plan}");
            }
        };
        {
            let conn = backend.get_connection().unwrap();
            conn.execute_batch("BEGIN IMMEDIATE; UPDATE search_index SET composite_slot=NULL WHERE param_name='code-value-concept'").unwrap();
            let error = backend
                .search_with_connection(&conn, &tenant, &query, None)
                .unwrap_err();
            assert!(error.to_string().contains("$reindex"), "{error}");
            for (missing, expected) in [(false, 1), (true, 0)] {
                for count in [0, 100] {
                    let mut presence = query.clone();
                    presence.parameters[0].modifier = Some(crate::types::SearchModifier::Missing);
                    presence.parameters[0].values = vec![SearchValue::eq(missing.to_string())];
                    presence.count = Some(count);
                    let result = backend
                        .search_with_connection(&conn, &tenant, &presence, None)
                        .unwrap();
                    assert_eq!(
                        result.resources.items.len(),
                        if count == 0 { 0 } else { expected }
                    );
                }
            }
            explain_real_probe(&conn);
            let stale_sql = legacy_composite_probe_sql(&conn, false).unwrap();
            let definition: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name='idx_search_composite_legacy'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            conn.execute("DROP INDEX idx_search_composite_legacy", [])
                .unwrap();
            assert!(
                run_legacy_composite_probe(
                    &conn,
                    &stale_sql,
                    tenant.tenant_id().as_str(),
                    "Observation",
                    "code-value-concept"
                )
                .unwrap()
                .is_some(),
                "a concurrent bulk drop between metadata and prepare retains the guard"
            );
            assert!(
                run_legacy_composite_probe(
                    &conn,
                    "SELECT invalid_column FROM search_index",
                    tenant.tenant_id().as_str(),
                    "Observation",
                    "code-value-concept"
                )
                .is_err(),
                "other SQL errors are not hidden by fallback"
            );
            conn.execute(&definition, []).unwrap();
            conn.execute_batch("ROLLBACK").unwrap();
            explain_real_probe(&conn);
        }
        assert!(backend.search(&tenant, &query).await.is_ok());
        for component in &mut query.parameters[0].components {
            component.param_type = SearchParamType::Reference;
        }
        for mode in [
            crate::types::ContainedMode::Off,
            crate::types::ContainedMode::On,
            crate::types::ContainedMode::Both,
        ] {
            query.contained = mode;
            let error = backend
                .search(&tenant, &query)
                .await
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("code-value-concept") && error.contains("unsupported repeated"),
                "{error}"
            );
            assert!(
                !error.contains("$reindex"),
                "an unsupported shape cannot be repaired by reindex: {error}"
            );
        }
    }

    fn composite_query(v: &str) -> SearchQuery {
        use crate::types::{CompositeSearchComponent, SearchParamType, SearchValue};
        SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code-value-quantity".to_string(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq(v)],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".to_string(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value-quantity".to_string(),
                },
            ],
            ..Default::default()
        })
    }

    #[tokio::test]
    async fn a_composite_value_with_the_wrong_number_of_parts_is_refused() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        let message_of = |err: StorageError| match err {
            StorageError::Search(SearchError::InvalidComposite { message }) => message,
            other => panic!("expected InvalidComposite, got {other:?}"),
        };

        let err = backend
            .search(&tenant, &composite_query("8302-2"))
            .await
            .expect_err("one part for two components");
        let message = message_of(err);
        assert!(message.contains("has 1 component(s)"), "{message}");
        assert!(message.contains("expects 2"), "{message}");

        let err = backend
            .search_count(&tenant, &composite_query("8302-2"))
            .await
            .expect_err("search_count is gated too");
        let message = message_of(err);
        assert!(message.contains("has 1 component(s)"), "{message}");
        assert!(message.contains("expects 2"), "{message}");

        let err = backend
            .search(&tenant, &composite_query("8302-2$5$6"))
            .await
            .expect_err("three parts for two components");
        assert!(message_of(err).contains("has 3 component(s)"));

        let mut contained = composite_query("8302-2");
        contained.contained = crate::types::ContainedMode::On;
        let err = backend
            .search(&tenant, &contained)
            .await
            .expect_err("the contained path is gated too");
        let message = message_of(err);
        assert!(message.contains("has 1 component(s)"), "{message}");
        assert!(message.contains("expects 2"), "{message}");

        let err = backend
            .search_ids(&tenant, &composite_query("8302-2"))
            .await
            .expect_err("search_ids is gated too");
        let message = message_of(err);
        assert!(message.contains("has 1 component(s)"), "{message}");
        assert!(message.contains("expects 2"), "{message}");

        // Positive control: the right number of parts is still a search.
        backend
            .search(&tenant, &composite_query("8302-2$gt150"))
            .await
            .expect("two parts for two components");
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

    /// Creates Patients `cp-1..cp-n` (inclusive) in the given tenant.
    async fn create_cursor_paging_patients(
        backend: &SqliteBackend,
        tenant: &TenantContext,
        n: u32,
    ) {
        for i in 1..=n {
            backend
                .create(
                    tenant,
                    "Patient",
                    json!({
                        "resourceType": "Patient",
                        "id": format!("cp-{i}"),
                        "name": [{"family": "CursorPaging"}]
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
    }

    /// Collects the resource ids of a search result's page, in page order.
    fn page_ids(result: &SearchResult) -> Vec<String> {
        result
            .resources
            .items
            .iter()
            .map(|r| r.id().to_string())
            .collect()
    }

    /// Walks forward through 7 Patients 3 at a time (no explicit `_sort`),
    /// then walks all the way back via `previous_cursor` and confirms every
    /// page is reproduced exactly, including the page-3-to-page-2 hop that a
    /// naive truncate-after-reverse implementation gets wrong (#1079).
    #[tokio::test]
    async fn test_cursor_paging_round_trip_previous() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        create_cursor_paging_patients(&backend, &tenant, 7).await;

        let query = SearchQuery::new("Patient").with_count(3);

        let page1 = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page1.resources.items.len(), 3);
        assert!(!page1.resources.page_info.has_previous);
        assert!(page1.resources.page_info.previous_cursor.is_none());
        assert!(page1.resources.page_info.has_next);
        assert!(page1.resources.page_info.next_cursor.is_some());

        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page2.resources.items.len(), 3);
        assert!(page2.resources.page_info.has_previous);
        assert!(page2.resources.page_info.previous_cursor.is_some());
        assert!(page2.resources.page_info.has_next);

        let page3 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page3.resources.items.len(), 1);
        assert!(!page3.resources.page_info.has_next);
        assert!(page3.resources.page_info.next_cursor.is_none());
        assert!(page3.resources.page_info.previous_cursor.is_some());

        let page1_ids = page_ids(&page1);
        let page2_ids = page_ids(&page2);
        let page3_ids = page_ids(&page3);
        let mut all_ids = page1_ids.clone();
        all_ids.extend(page2_ids.clone());
        all_ids.extend(page3_ids.clone());
        let mut unique_ids = all_ids.clone();
        unique_ids.sort();
        unique_ids.dedup();
        assert_eq!(unique_ids.len(), 7, "all 7 ids must be distinct");

        // Walk back: page 3 -> page 2 must be exact, including order. This is
        // the case a naive truncate-after-reverse gets wrong: it drops the
        // nearest hit instead of the farthest one.
        let back2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page3.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back2), page2_ids);
        assert!(back2.resources.page_info.has_previous);
        assert!(back2.resources.page_info.has_next);
        assert!(back2.resources.page_info.next_cursor.is_some());

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), page1_ids);
        assert!(!back1.resources.page_info.has_previous);
        assert!(back1.resources.page_info.previous_cursor.is_none());
        assert!(back1.resources.page_info.has_next);

        let again2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&again2), page2_ids);
    }

    /// Same round trip, but with an explicit `_sort=_id` ascending so the
    /// exact page contents (not just their distinctness) can be asserted.
    #[tokio::test]
    async fn test_cursor_paging_round_trip_previous_with_sort() {
        use crate::types::{SortDirection, SortDirective};

        let backend = create_test_backend();
        let tenant = create_test_tenant();
        create_cursor_paging_patients(&backend, &tenant, 7).await;

        let query = SearchQuery::new("Patient")
            .with_count(3)
            .with_sort(SortDirective {
                parameter: "_id".to_string(),
                direction: SortDirection::Ascending,
                param_type: None,
            });

        let page1 = backend.search(&tenant, &query).await.unwrap();
        assert_eq!(page_ids(&page1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(!page1.resources.page_info.has_previous);
        assert!(page1.resources.page_info.previous_cursor.is_none());
        assert!(page1.resources.page_info.has_next);

        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page2), vec!["cp-4", "cp-5", "cp-6"]);
        assert!(page2.resources.page_info.has_previous);
        assert!(page2.resources.page_info.has_next);

        let page3 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page3), vec!["cp-7"]);
        assert!(!page3.resources.page_info.has_next);
        assert!(page3.resources.page_info.next_cursor.is_none());
        assert!(page3.resources.page_info.previous_cursor.is_some());

        let back2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page3.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back2), vec!["cp-4", "cp-5", "cp-6"]);
        assert!(back2.resources.page_info.has_previous);
        assert!(back2.resources.page_info.has_next);
        assert!(back2.resources.page_info.next_cursor.is_some());

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(!back1.resources.page_info.has_previous);
        assert!(back1.resources.page_info.previous_cursor.is_none());
        assert!(back1.resources.page_info.has_next);

        let again2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(back1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&again2), vec!["cp-4", "cp-5", "cp-6"]);
    }

    /// Backward from page 2 of a 4-item, count-3 listing has no extra row
    /// beyond page 1, so `has_previous` must be false and no
    /// `previous_cursor` is produced.
    #[tokio::test]
    async fn test_cursor_paging_backward_from_page_two_has_no_previous() {
        use crate::types::{SortDirection, SortDirective};

        let backend = create_test_backend();
        let tenant = create_test_tenant();
        create_cursor_paging_patients(&backend, &tenant, 4).await;

        let query = SearchQuery::new("Patient")
            .with_count(3)
            .with_sort(SortDirective {
                parameter: "_id".to_string(),
                direction: SortDirection::Ascending,
                param_type: None,
            });

        let page1 = backend.search(&tenant, &query).await.unwrap();
        let page2 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page1.resources.page_info.next_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&page2), vec!["cp-4"]);

        let back1 = backend
            .search(
                &tenant,
                &query
                    .clone()
                    .with_cursor(page2.resources.page_info.previous_cursor.clone().unwrap()),
            )
            .await
            .unwrap();
        assert_eq!(page_ids(&back1), vec!["cp-1", "cp-2", "cp-3"]);
        assert!(!back1.resources.page_info.has_previous);
        assert!(back1.resources.page_info.has_next);
        assert!(back1.resources.page_info.next_cursor.is_some());
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

    #[tokio::test]
    async fn test_resolve_includes_renamed_param_uses_registry() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();

        // The `organization` search parameter maps to `managingOrganization`,
        // not a field literally named `organization` — only the registry-
        // driven resolver (FHIRPath expression, not `content.get(search_param)`)
        // can follow it.
        backend
            .create_or_update(
                &tenant,
                "Organization",
                "org-1",
                json!({"id": "org-1", "name": "Acme Clinic"}),
                FhirVersion::default(),
            )
            .await
            .unwrap();
        let (patient, _) = backend
            .create_or_update(
                &tenant,
                "Patient",
                "p1",
                json!({
                    "id": "p1",
                    "managingOrganization": {"reference": "Organization/org-1"}
                }),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        let include = IncludeDirective {
            include_type: crate::types::IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: None,
            iterate: false,
        };

        let included = backend
            .resolve_includes(&tenant, std::slice::from_ref(&patient), &[include])
            .await
            .unwrap();

        assert_eq!(included.len(), 1);
        assert_eq!(included[0].resource_type(), "Organization");
        assert_eq!(included[0].id(), "org-1");

        // A target-type filter that doesn't match the referenced type yields
        // nothing.
        let include_wrong_target = IncludeDirective {
            include_type: crate::types::IncludeType::Include,
            source_type: "Patient".to_string(),
            search_param: "organization".to_string(),
            target_type: Some("Practitioner".to_string()),
            iterate: false,
        };

        let included = backend
            .resolve_includes(&tenant, &[patient], &[include_wrong_target])
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
    // Chained Search Tests
    // ========================================================================

    /// #1930: the reference-value read reaches `search_index` by
    /// `resource_key` through `idx_search_composite`, never by scanning the
    /// table or every row of the reference parameter.
    #[test]
    fn reference_values_probe_the_composite_index_by_resource_key() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        let query = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "code".to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq("http://loinc.org|8302-2")],
            chain: vec![],
            components: vec![],
        });
        let (sql, params) = reference_values_statement(&tenant, &query, "patient");
        let conn = backend.get_connection().unwrap();
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let plan: Vec<String> = stmt
            .query_map(param_refs.as_slice(), |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            plan.iter().any(|line| line.starts_with("SEARCH si")
                && line.contains("idx_search_composite")
                && line.contains("resource_key=?")),
            "{plan:#?}"
        );
        assert!(
            !plan.iter().any(|line| line.starts_with("SCAN si")),
            "{plan:#?}"
        );
    }

    /// #1955: a reference search (`Observation?subject=Patient/x`) seeks the
    /// partial `idx_search_reference` index. Without `value_reference IS NOT
    /// NULL` beside the `=`-or-`_history`-range test, the planner does not
    /// prove the partial index's `WHERE` and reads every `subject` row of the
    /// type instead: 28 s instead of 1 s on 454k Observations.
    #[test]
    fn reference_search_seeks_the_partial_reference_index() {
        let backend = create_test_backend();
        let subject = |values: Vec<SearchValue>| {
            SearchQuery::new("Observation").with_parameter(SearchParameter {
                name: "subject".to_string(),
                param_type: SearchParamType::Reference,
                modifier: None,
                values,
                chain: vec![],
                components: vec![],
            })
        };
        // Patient/p1/Observation: membership through any compartment param.
        let mut compartment = SearchQuery::new("Observation");
        compartment.compartment = Some(crate::types::CompartmentMembership {
            params: vec!["subject".to_string(), "performer".to_string()],
            reference: "Patient/p1".to_string(),
        });
        for query in [
            subject(vec![SearchValue::eq("Patient/p1")]),
            subject(vec![
                SearchValue::eq("Patient/p1"),
                SearchValue::eq("Patient/p2"),
            ]),
            compartment,
        ] {
            let fragment = QueryBuilder::new("t", "Observation")
                .with_param_offset(2)
                .build(&query);
            let sql = format!(
                "EXPLAIN QUERY PLAN SELECT id FROM resources \
             WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 \
             AND rowid IN ({}) ORDER BY last_updated DESC, id ASC LIMIT 101",
                fragment.sql
            );
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![
                Box::new("t".to_string()),
                Box::new("Observation".to_string()),
            ];
            for param in &fragment.params {
                if let SqlParam::String(value) = param {
                    params.push(Box::new(value.clone()));
                }
            }
            let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let conn = backend.get_connection().unwrap();
            let mut stmt = conn.prepare(&sql).unwrap();
            let plan: Vec<String> = stmt
                .query_map(param_refs.as_slice(), |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            // The seek must be bounded on the value: an equality, or a range
            // with both ends. `value_reference>?` alone reads to the end of
            // the parameter's rows.
            assert!(
                plan.iter().any(|line| line.contains("idx_search_reference")
                    && (line.contains("value_reference=?")
                        || (line.contains("value_reference>")
                            && line.contains("value_reference<")))),
                "{plan:#?}"
            );
        }
    }

    thread_local! {
        static TRACED_SQL: std::cell::RefCell<Vec<String>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// `Connection::trace_v2` callback: keeps each statement as it starts,
    /// with its bound values written in.
    fn record_traced_sql(event: rusqlite::trace::TraceEvent<'_>) {
        if let rusqlite::trace::TraceEvent::Stmt(statement, _) = event
            && let Some(sql) = statement.expanded_sql()
        {
            TRACED_SQL.with(|traced| traced.borrow_mut().push(sql));
        }
    }

    /// #1506: a filtered one-row page on the default order reads each match
    /// by rowid, where SQLite would probe `idx_resources_live_type` first, and
    /// a single date parameter is not wrapped in a `SELECT DISTINCT` that
    /// re-probes `idx_search_composite` per key; `le`'s two ranges are a
    /// `UNION ALL`. A longer page keeps the walk of `idx_resources_reindex`
    /// that stops at the page, and both pages start with the same resource.
    #[tokio::test]
    async fn a_one_row_page_reads_its_matches_by_rowid() {
        use crate::types::{SearchParamType, SearchPrefix};
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        for (id, start) in [
            ("e1", "1990-01-01"),
            ("e2", "1999-06-01"),
            ("e3", "2005-01-01"),
        ] {
            backend
                .create(
                    &tenant,
                    "Encounter",
                    json!({"resourceType": "Encounter", "id": id, "status": "finished",
                           "class": {"code": "AMB"}, "period": {"start": start}}),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let query = |count| {
            let mut query = SearchQuery::new("Encounter").with_parameter(SearchParameter {
                name: "date".to_string(),
                param_type: SearchParamType::Date,
                modifier: None,
                values: vec![SearchValue::new(SearchPrefix::Le, "2000")],
                chain: vec![],
                components: vec![],
            });
            query.count = Some(count);
            query
        };

        let conn = backend.get_connection().unwrap();
        conn.trace_v2(
            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
            Some(record_traced_sql),
        );
        let mut first = Vec::new();
        for (count, read) in [
            (1, "USING INTEGER PRIMARY KEY"),
            (10, "idx_resources_reindex"),
        ] {
            TRACED_SQL.with(|traced| traced.borrow_mut().clear());
            let result = backend
                .search_with_connection_probe(&conn, &tenant, &query(count), None, false)
                .unwrap();
            let ids: Vec<&str> = result.resources.items.iter().map(|r| r.id()).collect();
            assert_eq!(ids.len(), count.min(2) as usize, "{ids:?}");
            first.push(ids[0].to_string());

            let page = TRACED_SQL
                .with(|traced| {
                    traced
                        .borrow()
                        .iter()
                        .find(|sql| sql.starts_with("SELECT id, version_id, data"))
                        .cloned()
                })
                .expect("the page statement");
            assert!(!page.contains("SELECT DISTINCT"), "{page}");
            assert_eq!(page.matches(" UNION ALL ").count(), 1, "{page}");
            let plan: Vec<String> = conn
                .prepare(&format!("EXPLAIN QUERY PLAN {page}"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(
                plan.iter()
                    .any(|line| line.starts_with("SEARCH resources") && line.contains(read)),
                "_count={count}: {plan:#?}"
            );
            assert!(
                !plan
                    .iter()
                    .any(|line| line.contains("idx_resources_live_type")),
                "_count={count}: {plan:#?}"
            );
            assert!(
                plan.iter()
                    .any(|line| line.contains("idx_search_date_start")),
                "{plan:#?}"
            );
        }
        conn.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
        assert_eq!(first[0], first[1]);
    }

    /// #1955: a search on positive parameters takes its candidates straight
    /// from the value indexes. It no longer re-reads every `search_index` row
    /// of every candidate through `idx_search_composite` to de-duplicate keys.
    #[test]
    fn positive_filter_candidates_skip_the_composite_index_probe() {
        let backend = create_test_backend();
        let token = |name: &str, value: &str| SearchParameter {
            name: name.to_string(),
            param_type: SearchParamType::Token,
            modifier: None,
            values: vec![SearchValue::eq(value)],
            chain: vec![],
            components: vec![],
        };
        for query in [
            SearchQuery::new("Observation").with_parameter(token("category", "laboratory")),
            SearchQuery::new("Observation")
                .with_parameter(token("category", "laboratory"))
                .with_parameter(token("code", "http://loinc.org|8302-2")),
        ] {
            let fragment = QueryBuilder::new("t", "Observation")
                .with_param_offset(2)
                .build(&query);
            let sql = format!(
                "EXPLAIN QUERY PLAN SELECT id FROM resources \
                 WHERE tenant_id = ?1 AND resource_type = ?2 AND is_deleted = 0 \
                 AND rowid IN ({}) ORDER BY last_updated DESC, id ASC LIMIT 101",
                fragment.sql
            );
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![
                Box::new("t".to_string()),
                Box::new("Observation".to_string()),
            ];
            for param in &fragment.params {
                if let SqlParam::String(value) = param {
                    params.push(Box::new(value.clone()));
                }
            }
            let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let conn = backend.get_connection().unwrap();
            let mut stmt = conn.prepare(&sql).unwrap();
            let plan: Vec<String> = stmt
                .query_map(param_refs.as_slice(), |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert!(
                plan.iter().any(|line| line.contains("idx_search_token")),
                "{plan:#?}"
            );
            assert!(
                !plan
                    .iter()
                    .any(|line| line.contains("idx_search_composite")),
                "{plan:#?}"
            );
        }
    }

    /// #1930: search, id-only search and count wait for SQLite on a blocking
    /// thread. With the pool's only connection held, the first poll of each
    /// is pending instead of parking the async worker until the pool times
    /// out, so a request timeout or a dropped request can take effect.
    #[tokio::test]
    async fn search_waits_for_sqlite_off_the_async_worker() {
        let mut config = crate::backends::sqlite::backend::SqliteBackendConfig::default();
        config.max_connections = 1;
        config.min_connections = 1;
        config.connection_timeout_ms = 2_000;
        let backend = SqliteBackend::with_config(":memory:", config).unwrap();
        backend.init_schema().unwrap();
        let tenant = create_test_tenant();
        let query = SearchQuery::new("Patient");

        let held = backend.get_connection().unwrap();
        let mut search = Box::pin(backend.search(&tenant, &query));
        let mut ids = Box::pin(backend.search_ids(&tenant, &query));
        let mut count = Box::pin(backend.search_count(&tenant, &query));
        assert!(futures::poll!(&mut search).is_pending(), "search");
        assert!(futures::poll!(&mut ids).is_pending(), "search_ids");
        assert!(futures::poll!(&mut count).is_pending(), "search_count");
        drop(held);

        assert!(search.await.unwrap().resources.items.is_empty());
        assert!(ids.await.unwrap().items.is_empty());
        assert_eq!(count.await.unwrap(), 0);
    }

    /// `search()` reads neither `SearchParameter::chain` nor
    /// `SearchQuery::reverse_chains`, so a query that still carries either
    /// must be refused, not answered: an unresolved `_has` used to match every
    /// Patient, and a forward chain was read as a reference to an id (#1389).
    #[tokio::test]
    async fn search_refuses_unresolved_chains() {
        use crate::types::{ChainedParameter, ReverseChainedParameter, SearchParamType};

        let backend = create_test_backend();
        let tenant = create_test_tenant();
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
                "Observation",
                json!({"id": "o1", "status": "final", "subject": {"reference": "Patient/p1"}}),
                FhirVersion::default(),
            )
            .await
            .unwrap();

        // Observation?subject:Patient.name=Smith
        let forward = SearchQuery::new("Observation").with_parameter(SearchParameter {
            name: "subject".to_string(),
            param_type: SearchParamType::Reference,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![ChainedParameter {
                reference_param: "subject".to_string(),
                target_type: Some("Patient".to_string()),
                target_param: "name".to_string(),
            }],
            ..Default::default()
        });
        // Patient?_has:Observation:subject:status=cancelled — no match, yet
        // it used to return p1.
        let mut reverse = SearchQuery::new("Patient");
        reverse
            .reverse_chains
            .push(ReverseChainedParameter::terminal(
                "Observation",
                "subject",
                "status",
                SearchValue::eq("cancelled"),
            ));

        for err in [
            backend.search(&tenant, &forward).await.err(),
            backend.search_count(&tenant, &forward).await.err(),
        ] {
            match err {
                Some(StorageError::Search(SearchError::ChainedSearchNotSupported { chain })) => {
                    assert!(chain.contains("subject:Patient.name"), "{chain}")
                }
                other => panic!("expected ChainedSearchNotSupported, got {other:?}"),
            }
        }
        for err in [
            backend.search(&tenant, &reverse).await.err(),
            backend.search_count(&tenant, &reverse).await.err(),
        ] {
            assert!(
                matches!(
                    err,
                    Some(StorageError::Search(SearchError::ReverseChainNotSupported))
                ),
                "expected ReverseChainNotSupported, got {err:?}"
            );
        }
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

    // ---- _contained / _containedType search ----

    use crate::types::{ContainedMode, ContainedReturn, SearchParamType, SearchValue};

    /// Seeds an Observation containing a Patient named "Smith", plus a top-level
    /// Patient also named "Smith".
    async fn seed_contained(backend: &SqliteBackend, tenant: &TenantContext) {
        backend
            .create(
                tenant,
                "Observation",
                json!({
                    "resourceType": "Observation",
                    "id": "obs1",
                    "status": "final",
                    "code": { "coding": [{ "system": "http://loinc.org", "code": "1234-5" }] },
                    "subject": { "reference": "#p1" },
                    "contained": [{
                        "resourceType": "Patient",
                        "id": "p1",
                        "name": [{ "family": "Smith", "given": ["Contained"] }],
                        "gender": "male"
                    }]
                }),
                helios_fhir::FhirVersion::default(),
            )
            .await
            .unwrap();
        backend
            .create(
                tenant,
                "Patient",
                json!({ "resourceType": "Patient", "id": "top1", "name": [{ "family": "Smith" }] }),
                helios_fhir::FhirVersion::default(),
            )
            .await
            .unwrap();
    }

    fn name_query(mode: ContainedMode, ret: ContainedReturn) -> SearchQuery {
        let mut q = SearchQuery::new("Patient");
        q.contained = mode;
        q.contained_return = ret;
        q.parameters.push(SearchParameter {
            name: "name".to_string(),
            param_type: SearchParamType::String,
            modifier: None,
            values: vec![SearchValue::eq("Smith")],
            chain: vec![],
            components: vec![],
        });
        q
    }

    #[tokio::test]
    async fn contained_off_excludes_contained_resources() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        seed_contained(&backend, &tenant).await;

        // Default (_contained=off): only the top-level Patient matches.
        let result = backend
            .search(
                &tenant,
                &name_query(ContainedMode::Off, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        assert_eq!(urls, vec!["Patient/top1"]);
    }

    #[tokio::test]
    async fn contained_on_returns_container_by_default() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &name_query(ContainedMode::On, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        assert_eq!(urls, vec!["Observation/obs1"], "container is returned");
    }

    #[tokio::test]
    async fn contained_on_contained_type_returns_contained_resource() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &name_query(ContainedMode::On, ContainedReturn::Contained),
            )
            .await
            .unwrap();
        assert_eq!(result.resources.items.len(), 1);
        let r = &result.resources.items[0];
        assert_eq!(r.resource_type(), "Patient");
        assert_eq!(r.id(), "p1");
        assert_eq!(r.content()["name"][0]["given"][0], "Contained");
    }

    #[tokio::test]
    async fn contained_both_merges_top_level_and_container() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        seed_contained(&backend, &tenant).await;

        let result = backend
            .search(
                &tenant,
                &name_query(ContainedMode::Both, ContainedReturn::Container),
            )
            .await
            .unwrap();
        let mut urls: Vec<String> = result.resources.items.iter().map(|r| r.url()).collect();
        urls.sort();
        assert_eq!(urls, vec!["Observation/obs1", "Patient/top1"]);
    }

    #[tokio::test]
    async fn contained_respects_param_so_no_false_match() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        seed_contained(&backend, &tenant).await;

        // No contained Patient named "Jones" → no container match.
        let mut q = name_query(ContainedMode::On, ContainedReturn::Container);
        q.parameters[0].values = vec![SearchValue::eq("Jones")];
        let result = backend.search(&tenant, &q).await.unwrap();
        assert!(result.resources.items.is_empty());
    }

    // ---- bounded probe + first-page plan selection ----

    use crate::types::{CompositeSearchComponent, SearchPrefix};

    fn obs_param(
        name: &str,
        param_type: SearchParamType,
        prefix: SearchPrefix,
        value: &str,
    ) -> SearchParameter {
        SearchParameter {
            name: name.to_string(),
            param_type,
            modifier: None,
            values: vec![SearchValue::new(prefix, value)],
            chain: vec![],
            components: vec![],
        }
    }

    /// 2,300 Observations: `B2001` is indexed on exactly 2,001 of them and
    /// `B2000` on exactly 2,000, so a probe capped at 2,001 rows sees the
    /// boundary on either side.
    async fn probe_fixture() -> (SqliteBackend, TenantContext) {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        for i in 0..2300usize {
            let mut codings =
                vec![json!({"system": "http://loinc.org", "code": format!("C{}", i % 7)})];
            if i < 2001 {
                codings.push(json!({"system": "http://x.org", "code": "B2001"}));
            }
            if i < 2000 {
                codings.push(json!({"system": "http://x.org", "code": "B2000"}));
            }
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": format!("o{i:04}"),
                        "status": "final",
                        "category": [{"coding": [{
                            "system": "http://terminology.hl7.org/CodeSystem/observation-category",
                            "code": if i % 10 == 0 { "vital-signs" } else { "laboratory" }}]}],
                        "code": {"coding": codings},
                        "subject": {"reference": format!("Patient/p{}", i % 50)},
                        "effectiveDateTime": format!("{}-01-15T10:00:00Z", 2000 + i % 20),
                        "valueQuantity": {"value": (i % 500) as f64, "unit": "mg",
                            "system": "http://unitsofmeasure.org", "code": "mg"}
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        (backend, tenant)
    }

    fn probe_plan_of(
        backend: &SqliteBackend,
        tenant: &TenantContext,
        query: &SearchQuery,
    ) -> FirstPagePlan {
        let conn = backend.get_connection().unwrap();
        let fragment = QueryBuilder::new(tenant.tenant_id().as_str(), &query.resource_type)
            .with_param_offset(2)
            .build(query);
        choose_first_page_plan(
            &conn,
            tenant.tenant_id().as_str(),
            &query.resource_type,
            &fragment.sql,
            &fragment.params,
        )
    }

    /// Every page of `query` through the planner path (probe off) and the
    /// probe path, compared id by id.
    fn assert_same_pages(backend: &SqliteBackend, tenant: &TenantContext, query: &SearchQuery) {
        let conn = backend.get_connection().unwrap();
        let mut cursor_old: Option<String> = None;
        let mut cursor_new: Option<String> = None;
        let mut seen = 0usize;
        for page_no in 0..6 {
            let mut q_old = query.clone();
            q_old.cursor = cursor_old.clone();
            let mut q_new = query.clone();
            q_new.cursor = cursor_new.clone();
            let old = backend
                .search_with_connection_probe(&conn, tenant, &q_old, None, false)
                .unwrap();
            let new = backend
                .search_with_connection_probe(&conn, tenant, &q_new, None, true)
                .unwrap();
            let ids = |r: &SearchResult| -> Vec<String> {
                r.resources
                    .items
                    .iter()
                    .map(|x| x.id().to_string())
                    .collect()
            };
            assert_eq!(
                ids(&old),
                ids(&new),
                "page {page_no} of {:?}",
                query.parameters
            );
            assert_eq!(
                old.resources.page_info.has_next,
                new.resources.page_info.has_next
            );
            seen += ids(&new).len();
            cursor_old = old.resources.page_info.next_cursor.clone();
            cursor_new = new.resources.page_info.next_cursor.clone();
            if cursor_old.is_none() {
                break;
            }
        }
        assert!(
            seen > 0 || query.parameters.is_empty(),
            "{:?} matched nothing",
            query.parameters
        );
    }

    #[tokio::test]
    async fn probe_plan_gives_the_same_pages_as_the_planner() {
        let (backend, tenant) = probe_fixture().await;
        let mut shapes: Vec<SearchParameter> = vec![
            // token: selective, dense, boundary, rare
            obs_param("code", SearchParamType::Token, SearchPrefix::Eq, "C3"),
            obs_param(
                "code",
                SearchParamType::Token,
                SearchPrefix::Eq,
                "http://loinc.org|C5",
            ),
            obs_param("code", SearchParamType::Token, SearchPrefix::Eq, "B2000"),
            obs_param("code", SearchParamType::Token, SearchPrefix::Eq, "B2001"),
            obs_param(
                "category",
                SearchParamType::Token,
                SearchPrefix::Eq,
                "laboratory",
            ),
            obs_param(
                "category",
                SearchParamType::Token,
                SearchPrefix::Eq,
                "vital-signs",
            ),
            // quantity: dense, selective, empty
            obs_param(
                "value-quantity",
                SearchParamType::Quantity,
                SearchPrefix::Lt,
                "460",
            ),
            obs_param(
                "value-quantity",
                SearchParamType::Quantity,
                SearchPrefix::Gt,
                "450",
            ),
            obs_param(
                "value-quantity",
                SearchParamType::Quantity,
                SearchPrefix::Le,
                "20",
            ),
            // date: dense, selective, union (le/ge), empty
            obs_param(
                "date",
                SearchParamType::Date,
                SearchPrefix::Gt,
                "2005-01-01",
            ),
            obs_param(
                "date",
                SearchParamType::Date,
                SearchPrefix::Gt,
                "2018-01-01",
            ),
            obs_param("date", SearchParamType::Date, SearchPrefix::Le, "2003"),
            obs_param("date", SearchParamType::Date, SearchPrefix::Ge, "2010"),
            // reference
            obs_param(
                "subject",
                SearchParamType::Reference,
                SearchPrefix::Eq,
                "Patient/p7",
            ),
        ];
        // composite stays on the planner path but must still agree
        shapes.push(SearchParameter {
            name: "code-value-quantity".into(),
            param_type: SearchParamType::Composite,
            values: vec![SearchValue::eq("C3$gt100")],
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".into(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value-quantity".into(),
                },
            ],
            ..Default::default()
        });
        for param in shapes {
            for count in [2u32, 7, 50, 250] {
                let mut query = SearchQuery::new("Observation").with_parameter(param.clone());
                query.count = Some(count);
                assert_same_pages(&backend, &tenant, &query);
            }
        }
        // a value no row has: both plans return an empty page
        let mut empty = SearchQuery::new("Observation").with_parameter(obs_param(
            "code",
            SearchParamType::Token,
            SearchPrefix::Eq,
            "no-such-code",
        ));
        empty.count = Some(20);
        let conn = backend.get_connection().unwrap();
        for probe in [false, true] {
            let r = backend
                .search_with_connection_probe(&conn, &tenant, &empty, None, probe)
                .unwrap();
            assert!(r.resources.items.is_empty());
        }
    }

    fn cvq(values: &[&str]) -> SearchParameter {
        SearchParameter {
            name: "code-value-quantity".into(),
            param_type: SearchParamType::Composite,
            values: values.iter().map(|v| SearchValue::eq(*v)).collect(),
            components: vec![
                CompositeSearchComponent {
                    param_type: SearchParamType::Token,
                    param_name: "code".into(),
                },
                CompositeSearchComponent {
                    param_type: SearchParamType::Quantity,
                    param_name: "value-quantity".into(),
                },
            ],
            ..Default::default()
        }
    }

    /// The code-first composite pairing returns the pages, the page flags and
    /// the total of the whole-parameter `GROUP BY`, with the probe on and off,
    /// for a selective code, a code past the probe cap (the `EXISTS`-driven
    /// window), an empty one, an OR of values, and the shapes that keep the
    /// `GROUP BY` (`system|` without a code).
    #[tokio::test]
    async fn composite_pairing_gives_the_same_pages_as_the_group_by() {
        let (backend, tenant) = probe_fixture().await;
        let conn = backend.get_connection().unwrap();
        let ids = |r: &SearchResult| -> Vec<String> {
            r.resources
                .items
                .iter()
                .map(|x| x.id().to_string())
                .collect()
        };
        let mut paired_somewhere = false;
        for values in [
            vec!["C3$gt100"],
            vec!["http://loinc.org|C5$lt20"],
            vec!["B2001$ge0"],
            vec!["B2001$gt400"],
            vec!["B2000$lt3"],
            vec!["C1$gt1000"],
            vec!["no-such$gt1"],
            vec!["C3$gt480", "C5$lt3"],
            vec!["http://loinc.org|$gt490"],
            vec!["|C2$gt490"],
        ] {
            for count in [2u32, 7, 50] {
                for probe in [false, true] {
                    let mut query = SearchQuery::new("Observation").with_parameter(cvq(&values));
                    query.count = Some(count);
                    let mut cursor: Option<String> = None;
                    for page_no in 0..4 {
                        query.cursor = cursor.clone();
                        let old = backend
                            .search_with_connection_plan(&conn, &tenant, &query, None, probe, false)
                            .unwrap();
                        let new = backend
                            .search_with_connection_plan(&conn, &tenant, &query, None, probe, true)
                            .unwrap();
                        assert_eq!(
                            ids(&old),
                            ids(&new),
                            "{values:?} count {count} probe {probe} page {page_no}"
                        );
                        assert_eq!(
                            old.resources.page_info.has_next,
                            new.resources.page_info.has_next
                        );
                        paired_somewhere |= !ids(&new).is_empty();
                        cursor = old.resources.page_info.next_cursor.clone();
                        if cursor.is_none() {
                            break;
                        }
                    }
                }
            }
            let query = SearchQuery::new("Observation").with_parameter(cvq(&values));
            assert_eq!(
                search_count_on_plan(&conn, &tenant, &query, false).unwrap(),
                search_count_on_plan(&conn, &tenant, &query, true).unwrap(),
                "{values:?} total"
            );
        }
        assert!(paired_somewhere);
    }

    /// The pairing is chosen only for a token(code)+quantity composite; the
    /// wide one past the probe cap is `EXISTS`-driven, and a missing hinted
    /// index falls back to the `GROUP BY` instead of failing the search.
    #[tokio::test]
    async fn composite_pairing_plan_and_missing_index_fallback() {
        let (backend, tenant) = probe_fixture().await;
        let conn = backend.get_connection().unwrap();
        let build = |values: &[&str], rewrite: bool| {
            QueryBuilder::new(tenant.tenant_id().as_str(), "Observation")
                .with_param_offset(2)
                .with_composite_rewrite(rewrite)
                .build(&SearchQuery::new("Observation").with_parameter(cvq(values)))
        };
        assert!(
            build(&["C3$gt1"], true)
                .sql
                .contains("INDEXED BY idx_search_token")
        );
        assert!(!build(&["C3$gt1"], false).sql.contains("INDEXED BY"));
        assert!(
            !build(&["http://loinc.org|$gt1"], true)
                .sql
                .contains("INDEXED BY")
        );
        // An OR of values keeps the GROUP BY (the pairing measured slower there).
        assert!(
            !build(&["C3$gt1", "C5$gt1"], true)
                .sql
                .contains("INDEXED BY")
        );
        let wide = build(&["B2001$ge0"], true);
        assert_eq!(
            choose_first_page_plan(&conn, "test-tenant", "Observation", &wide.sql, &wide.params),
            FirstPagePlan::ExistsDriven
        );
        assert!(
            membership_exists_predicate(&wide.sql, "w.rk")
                .unwrap()
                .contains("c.resource_key = w.rk")
        );
        let narrow = build(&["C3$gt400"], true);
        assert_eq!(
            choose_first_page_plan(
                &conn,
                "test-tenant",
                "Observation",
                &narrow.sql,
                &narrow.params
            ),
            FirstPagePlan::ListDriven
        );

        let mut query = SearchQuery::new("Observation").with_parameter(cvq(&["C3$gt100"]));
        query.count = Some(5);
        let with_index = backend
            .search_with_connection(&conn, &tenant, &query, None)
            .unwrap();
        assert!(!with_index.resources.items.is_empty());
        conn.execute("DROP INDEX idx_search_token", []).unwrap();
        let without = backend
            .search_with_connection(&conn, &tenant, &query, None)
            .unwrap();
        let ids = |r: &SearchResult| -> Vec<String> {
            r.resources
                .items
                .iter()
                .map(|x| x.id().to_string())
                .collect()
        };
        assert_eq!(ids(&with_index), ids(&without));
        // Forcing the pairing against the missing index is the error the
        // fallback absorbs.
        let forced = backend.search_with_connection_plan(&conn, &tenant, &query, None, false, true);
        assert!(forced.unwrap_err().to_string().contains("no such index"));
        assert_eq!(
            search_count_on(&conn, &tenant, &query).unwrap(),
            search_count_on_plan(&conn, &tenant, &query, false).unwrap()
        );
    }

    /// A broad date search (`le`: a `UNION ALL` of two ranges) whose matches
    /// are all older than the newest 512 resources: the first window comes up
    /// empty, the 4,096 window fills the page, and the page is the planner's.
    #[tokio::test]
    async fn union_date_search_widens_the_recency_window() {
        let backend = create_test_backend();
        let tenant = create_test_tenant();
        for i in 0..2600usize {
            backend
                .create(
                    &tenant,
                    "Observation",
                    json!({
                        "resourceType": "Observation",
                        "id": format!("d{i:04}"),
                        "status": "final",
                        "code": {"coding": [{"system": "http://loinc.org", "code": "X"}]},
                        // the 2,050 oldest are the only matches of `le2000`
                        "effectiveDateTime": if i < 2050 { "1990-01-15T10:00:00Z" } else { "2020-01-15T10:00:00Z" },
                    }),
                    FhirVersion::default(),
                )
                .await
                .unwrap();
        }
        let query = SearchQuery::new("Observation").with_parameter(obs_param(
            "date",
            SearchParamType::Date,
            SearchPrefix::Le,
            "2000",
        ));
        assert_eq!(
            probe_plan_of(&backend, &tenant, &query),
            FirstPagePlan::ExistsDriven
        );
        let conn = backend.get_connection().unwrap();
        conn.trace_v2(
            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
            Some(record_traced_sql),
        );
        TRACED_SQL.with(|t| t.borrow_mut().clear());
        let probed = backend
            .search_with_connection_probe(&conn, &tenant, &query, None, true)
            .unwrap();
        let windows: Vec<String> = TRACED_SQL.with(|t| {
            t.borrow()
                .iter()
                .filter(|s| s.contains("INDEXED BY idx_resources_reindex"))
                .cloned()
                .collect()
        });
        assert_eq!(windows.len(), 2, "{windows:?}");
        assert!(windows[0].contains("LIMIT 512)") && windows[1].contains("LIMIT 4096)"));
        assert!(windows[1].contains(") OR EXISTS ("));
        let planned = backend
            .search_with_connection_probe(&conn, &tenant, &query, None, false)
            .unwrap();
        let ids = |r: &SearchResult| -> Vec<String> {
            r.resources
                .items
                .iter()
                .map(|x| x.id().to_string())
                .collect()
        };
        assert_eq!(ids(&probed), ids(&planned));
        assert_eq!(ids(&probed).len(), 100);
        assert!(probed.resources.page_info.has_next);
    }

    #[tokio::test]
    async fn probe_boundary_is_2001_index_rows() {
        let (backend, tenant) = probe_fixture().await;
        let plan = |value: &str| {
            let mut q = SearchQuery::new("Observation").with_parameter(obs_param(
                "code",
                SearchParamType::Token,
                SearchPrefix::Eq,
                value,
            ));
            q.count = Some(10);
            probe_plan_of(&backend, &tenant, &q)
        };
        assert_eq!(plan("B2000"), FirstPagePlan::ListDriven, "2000 rows");
        assert_eq!(plan("B2001"), FirstPagePlan::ExistsDriven, "2001 rows");
        assert_eq!(plan("C3"), FirstPagePlan::ListDriven);
        assert_eq!(plan("no-such-code"), FirstPagePlan::ListDriven);
        // composites, `:not` and two parameters never take the probe
        let mut two = SearchQuery::new("Observation")
            .with_parameter(obs_param(
                "code",
                SearchParamType::Token,
                SearchPrefix::Eq,
                "B2001",
            ))
            .with_parameter(obs_param(
                "category",
                SearchParamType::Token,
                SearchPrefix::Eq,
                "laboratory",
            ));
        two.count = Some(10);
        assert_eq!(
            probe_plan_of(&backend, &tenant, &two),
            FirstPagePlan::Planner
        );
    }

    #[tokio::test]
    async fn probe_picks_the_measured_plans_for_the_page_statement() {
        let (backend, tenant) = probe_fixture().await;
        let conn = backend.get_connection().unwrap();
        conn.trace_v2(
            rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
            Some(record_traced_sql),
        );
        for (value, needle) in [
            ("B2000", "resources NOT INDEXED"),
            (
                "B2001",
                "EXISTS (SELECT 1 FROM search_index INDEXED BY idx_search_composite",
            ),
        ] {
            TRACED_SQL.with(|t| t.borrow_mut().clear());
            let mut q = SearchQuery::new("Observation").with_parameter(obs_param(
                "code",
                SearchParamType::Token,
                SearchPrefix::Eq,
                value,
            ));
            q.count = Some(10);
            backend
                .search_with_connection_probe(&conn, &tenant, &q, None, true)
                .unwrap();
            let page = TRACED_SQL
                .with(|t| {
                    t.borrow()
                        .iter()
                        .find(|s| s.starts_with("SELECT id, version_id, data"))
                        .cloned()
                })
                .expect("the page statement");
            assert!(page.contains(needle), "{value}: {page}");
        }
        conn.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
    }
}
