//! SQLite implementation of [`ValueSetOperations`].
//!
//! ## Expansion strategy
//!
//! Expansion is computed lazily on the first `$expand` call and cached in the
//! `value_set_expansions` table.  Subsequent calls for the same ValueSet are
//! served from the cache.  The cache is invalidated (deleted) whenever the
//! ValueSet or any referenced CodeSystem is updated or deleted.
//!
//! ### Compose support
//!
//! * `compose.include[].system` — required in every include clause.
//! * `compose.include[].concept[]` — explicit code list; when absent, all
//!   codes from the referenced system are included.
//! * `compose.exclude[]` — removes specific `(system, code)` pairs after all
//!   includes have been resolved.
//!
//! ### Implicit ValueSets
//!
//! When the requested URL does not match any `value_sets` row, the backend
//! checks whether a CodeSystem carries `"valueSet": "<url>"`.  If found, an
//! on-the-fly expansion of all codes in that CodeSystem is returned (FHIR R5
//! §4.8.7).  Implicit expansions are not cached because they have no
//! corresponding row in `value_sets`.
//!
//! ### Hierarchical expansion
//!
//! When `ExpandRequest::hierarchical` is `Some(true)`, the flat expansion is
//! restructured into a tree using the pre-materialized `concept_hierarchy`
//! table.  Pagination is skipped in tree mode; the full tree is always
//! returned.
//!
//! ### Pagination
//!
//! `count` (page size) and `offset` (zero-based start) are applied in-memory
//! after filtering.  The `total` field in the response always reflects the
//! full (pre-pagination) count.

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use rusqlite::{Connection, OptionalExtension};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::ecl;
use crate::error::HtsError;
use crate::traits::ValueSetOperations;
use crate::types::{
    ExpandRequest, ExpandResponse, ExpansionContains, ResourceSearchQuery, ValidateCodeRequest,
    ValidateCodeResponse,
};

use super::SqliteTerminologyBackend;

#[async_trait]
impl ValueSetOperations for SqliteTerminologyBackend {
    /// Expand a value set by URL, returning all contained codes.
    ///
    /// Checks the `value_set_expansions` cache first. On cache miss, parses
    /// `compose_json`, queries `concepts` for matching codes, populates the
    /// cache, then returns the (paginated) result.
    async fn expand(
        &self,
        _ctx: &TenantContext,
        req: ExpandRequest,
    ) -> Result<ExpandResponse, HtsError> {
        if req.url.is_none() && req.value_set.is_none() {
            return Err(HtsError::InvalidRequest(
                "Missing required parameter: url (ValueSet canonical URL)".into(),
            ));
        }

        let pool = self.pool().clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            // Accumulates FHIR expansion warnings for unknown/skipped systems.
            // Only populated by the inline ValueSet path.
            let mut warnings: Vec<String> = Vec::new();

            let all_codes = if let Some(vs_resource) = req.value_set {
                // Inline ValueSet: extract compose and expand directly.
                // Systems not in the DB push a warning and are skipped; callers
                // receive partial results plus `expansion.parameter` warnings.
                let compose = &vs_resource["compose"];
                let codes = if let Some(filter) = req.filter.as_deref() {
                    expand_inline_filtered(&conn, compose, filter, &mut warnings)?
                } else {
                    let compose_str = compose.to_string();
                    // Cache inline compose expansions so that repeated requests for
                    // the same compose (e.g. ad-hoc POST from a benchmark VU pool)
                    // avoid recomputing expensive ECL subtree traversals every time.
                    // Key format: "inline-compose:<fnv64-hex>" — stored in the same
                    // implicit_expansion_cache table used for ?fhir_vs expansions.
                    let cache_key =
                        format!("inline-compose:{:016x}", fnv64(compose_str.as_bytes()));

                    let from_cache: Option<Vec<ExpansionContains>> = {
                        let exists: bool = conn
                            .query_row(
                                "SELECT EXISTS(\
                                     SELECT 1 FROM implicit_expansion_cache \
                                     WHERE url = ?1 LIMIT 1)",
                                [&cache_key],
                                |r| r.get(0),
                            )
                            .map_err(|e| HtsError::StorageError(e.to_string()))?;
                        if exists {
                            let mut stmt = conn
                                .prepare_cached(
                                    "SELECT system_url, code, display \
                                     FROM implicit_expansion_cache \
                                     WHERE url = ?1 \
                                     ORDER BY system_url, code",
                                )
                                .map_err(|e| HtsError::StorageError(e.to_string()))?;
                            let rows = stmt
                                .query_map([&cache_key], |r| {
                                    Ok(ExpansionContains {
                                        system: r.get(0)?,
                                        code: r.get(1)?,
                                        display: r.get(2)?,
                                        inactive: None,
                                        contains: vec![],
                                    })
                                })
                                .map_err(|e| HtsError::StorageError(e.to_string()))?
                                .collect::<Result<Vec<_>, _>>()
                                .map_err(|e| HtsError::StorageError(e.to_string()))?;
                            Some(rows)
                        } else {
                            None
                        }
                    };

                    if let Some(cached) = from_cache {
                        cached
                    } else {
                        // ── BFS fast path for simple hierarchy composes ───────────────
                        // When the compose is a single include with a single is-a or
                        // descendent-of filter (e.g. EX02: descendent-of Disease), use
                        // BFS to serve the requested page immediately instead of blocking
                        // on the full ECL expansion (which can take >30 s for large
                        // SNOMED hierarchies). We skip background cache population to
                        // avoid exhausting the r2d2 pool with long-running writes.
                        if let Some(count) = req.count.filter(|&c| c > 0) {
                            if let Some((sys_url, sys_id, root_code, include_root)) =
                                extract_simple_hierarchy_compose(&conn, compose, &mut warnings)?
                            {
                                let bfs_offset = req.offset.unwrap_or(0) as usize;
                                let page = bfs_isa_page(
                                    &conn,
                                    &sys_url,
                                    &sys_id,
                                    &root_code,
                                    include_root,
                                    bfs_offset,
                                    count as usize,
                                    None,
                                )?;
                                return Ok(ExpandResponse {
                                    total: None,
                                    offset: req.offset,
                                    contains: page,
                                    warnings,
                                });
                            }
                        }

                        let codes = compute_expansion(&conn, Some(&compose_str), &mut warnings)?;
                        // Only cache when all systems were resolved (no warnings mean
                        // the expansion is complete and safe to reuse).
                        if warnings.is_empty() {
                            let _ = populate_implicit_cache(&conn, &cache_key, &codes);
                        }
                        codes
                    }
                };

                // Total-miss guard: if every include clause was skipped (all
                // systems unknown), surface a NotFound rather than silently
                // returning an empty expansion with no explanation.
                let include_count = compose["include"].as_array().map_or(0, |a| a.len());
                if include_count > 0 && warnings.len() >= include_count {
                    return Err(HtsError::NotFound(
                        "None of the systems in the inline ValueSet compose could be resolved"
                            .into(),
                    ));
                }

                codes
            } else {
                let url = req.url.as_deref().unwrap();
                // Resolve expansion codes — either from an explicit ValueSet or from an
                // implicit one defined by `CodeSystem.valueSet`.
                match resolve_value_set(&conn, url, req.date.as_deref()) {
                    Ok((vs_id, compose_json)) => {
                        // Normal path: try the expansion cache first.
                        let cached = fetch_cache(&conn, &vs_id)?;
                        if cached.is_empty() {
                            // Fast page for paginated requests on large ValueSets
                            // (e.g. VSAC ValueSets with thousands of explicit codes).
                            // Serves offset+limit codes directly from the compose JSON
                            // without computing the full expansion, avoiding >30 s timeouts.
                            if let Some(count) = req.count.filter(|&c| c > 0) {
                                let page_offset = req.offset.unwrap_or(0) as usize;
                                if let Some((page, total)) = compose_page_fast(
                                    &conn,
                                    compose_json.as_deref(),
                                    page_offset,
                                    count as usize,
                                )? {
                                    return Ok(ExpandResponse {
                                        total: Some(total),
                                        offset: req.offset,
                                        contains: page,
                                        warnings: vec![],
                                    });
                                }
                            }
                            let codes =
                                compute_expansion(&conn, compose_json.as_deref(), &mut vec![])?;
                            populate_cache(&conn, &vs_id, &codes)?;
                            codes
                        } else {
                            cached
                        }
                    }
                    Err(HtsError::NotFound(_)) => {
                        // ── BFS fast path for cold-cache implicit ValueSets ───────────
                        // When the cache is empty and the client requested a bounded page
                        // (count > 0), serve it immediately from BFS/SQL traversal and
                        // spawn the full cache population in the background.  This avoids
                        // the >30 s timeout that a blocking recursive-CTE INSERT for
                        // large code systems (e.g. SNOMED CT ~350 K concepts) would cause.
                        let cache_populated: bool = conn
                            .query_row(
                                "SELECT EXISTS(\
                                     SELECT 1 FROM implicit_expansion_cache \
                                     WHERE url = ?1 LIMIT 1)",
                                [url],
                                |r| r.get(0),
                            )
                            .map_err(|e| HtsError::StorageError(e.to_string()))?;

                        if !cache_populated {
                            if let Some(count) = req.count.filter(|&c| c > 0) {
                                let cs_pat = if let Ok(cs_url) =
                                    find_cs_for_implicit_vs(&conn, url, req.date.as_deref())
                                {
                                    Some((cs_url, FhirVsPattern::AllConcepts))
                                } else {
                                    parse_fhir_vs_url(url)
                                };

                                if let Some((cs_url, pattern)) = cs_pat {
                                    let system_id: Option<String> = conn
                                        .query_row(
                                            "SELECT id FROM code_systems WHERE url = ?1",
                                            [&cs_url],
                                            |r| r.get(0),
                                        )
                                        .optional()
                                        .map_err(|e| HtsError::StorageError(e.to_string()))?;

                                    if let Some(system_id) = system_id {
                                        let filter_lower =
                                            req.filter.as_deref().map(|f| f.to_lowercase());
                                        let bfs_offset = req.offset.unwrap_or(0) as usize;
                                        let page = bfs_expand_page(
                                            &conn,
                                            &cs_url,
                                            &system_id,
                                            &pattern,
                                            bfs_offset,
                                            count as usize,
                                            filter_lower.as_deref(),
                                        )?;

                                        return Ok(ExpandResponse {
                                            total: None,
                                            offset: req.offset,
                                            contains: page,
                                            warnings: vec![],
                                        });
                                    }
                                }
                            }
                        }

                        // ── Blocking path: cache is warm, or count is None ────────────
                        ensure_implicit_cache(&conn, url, req.date.as_deref())?;

                        let filter_lower = req.filter.as_deref().map(|f| f.to_lowercase());
                        let sql_offset = i64::from(req.offset.unwrap_or(0));
                        let sql_limit = req.count.map(i64::from).unwrap_or(-1);

                        let total = implicit_cache_count(&conn, url, filter_lower.as_deref())?;

                        if req.count.is_none() {
                            if let Some(cap) = req.max_expansion_size {
                                if u64::from(total) > u64::from(cap) {
                                    return Err(HtsError::TooCostly(format!(
                                        "ValueSet expansion contains {} codes which exceeds \
                                         the server limit of {} (set \
                                         HTS_MAX_EXPANSION_SIZE to raise it)",
                                        total, cap
                                    )));
                                }
                            }
                        }

                        let page = implicit_cache_page(
                            &conn,
                            url,
                            filter_lower.as_deref(),
                            sql_limit,
                            sql_offset,
                        )?;

                        return Ok(ExpandResponse {
                            total: Some(total),
                            offset: req.offset,
                            contains: page,
                            warnings: vec![],
                        });
                    }
                    Err(e) => return Err(e),
                }
            };

            // Apply optional free-text filter (code or display substring match).
            let filtered: Vec<ExpansionContains> = if let Some(filter) = req.filter.as_deref() {
                let lower = filter.to_lowercase();
                all_codes
                    .into_iter()
                    .filter(|c| {
                        c.code.to_lowercase().contains(&lower)
                            || c.display
                                .as_deref()
                                .map(|d| d.to_lowercase().contains(&lower))
                                .unwrap_or(false)
                    })
                    .collect()
            } else {
                all_codes
            };

            // Hierarchical mode: build tree from the filtered flat list and
            // return without pagination (total = flat count, no offset/count).
            if req.hierarchical == Some(true) {
                let total = filtered.len() as u32;
                let tree = build_hierarchical_expansion(&conn, filtered)?;
                return Ok(ExpandResponse {
                    total: Some(total),
                    offset: None,
                    contains: tree,
                    warnings,
                });
            }

            let total = filtered.len() as u32;

            // Enforce the expansion size cap only when no explicit count (page size) was
            // requested. When count is set, the response is already bounded and the limit
            // would only reject valid paginated requests against large code systems.
            if req.count.is_none() {
                if let Some(limit) = req.max_expansion_size {
                    if u64::from(total) > u64::from(limit) {
                        return Err(HtsError::TooCostly(format!(
                            "ValueSet expansion contains {} codes which exceeds the server \
                             limit of {} (set HTS_MAX_EXPANSION_SIZE to raise it)",
                            total, limit
                        )));
                    }
                }
            }

            let offset = req.offset.unwrap_or(0) as usize;
            let count = req.count.map(|c| c as usize).unwrap_or(usize::MAX);

            let page: Vec<ExpansionContains> =
                filtered.into_iter().skip(offset).take(count).collect();

            Ok(ExpandResponse {
                total: Some(total),
                offset: req.offset,
                contains: page,
                warnings,
            })
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    /// Validate whether a code is a member of a value set.
    ///
    /// Triggers expansion if needed, then checks set membership.
    /// Returns `result = false` (not an error) when the value set or code is
    /// not found.
    async fn validate_code(
        &self,
        _ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError> {
        let url = req.url.clone().ok_or_else(|| {
            HtsError::InvalidRequest(
                "Missing required parameter: url (ValueSet canonical URL)".into(),
            )
        })?;

        let pool = self.pool().clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            // Resolve the expansion — try explicit ValueSet first, then the two
            // implicit-ValueSet fallbacks used by $expand.
            let all_codes: Vec<ExpansionContains> =
                match resolve_value_set(&conn, &url, req.date.as_deref()) {
                    Ok((vs_id, compose_json)) => {
                        let cached = fetch_cache(&conn, &vs_id)?;
                        if cached.is_empty() {
                            let codes =
                                compute_expansion(&conn, compose_json.as_deref(), &mut vec![])?;
                            populate_cache(&conn, &vs_id, &codes)?;
                            codes
                        } else {
                            cached
                        }
                    }
                    Err(HtsError::NotFound(_)) => {
                        // ?fhir_vs implicit ValueSet: do a targeted O(1)/O(depth) lookup
                        // instead of materializing all concepts (which times out for large
                        // code systems like SNOMED CT with ~350k concepts).
                        if let Some((cs_url, pattern)) = parse_fhir_vs_url(&url) {
                            let found = validate_fhir_vs(
                                &conn,
                                &cs_url,
                                &pattern,
                                &req.code,
                                req.system.as_deref(),
                            )?;
                            return finish_validate_code_response(
                                found,
                                &req.code,
                                &url,
                                req.display.as_deref(),
                            );
                        }

                        // Other implicit ValueSets (e.g. CodeSystem.valueSet link): use the
                        // expansion cache, then do an O(1) indexed SQL lookup.
                        ensure_implicit_cache(&conn, &url, req.date.as_deref())?;

                        let found = lookup_in_implicit_cache(
                            &conn,
                            &url,
                            &req.code,
                            req.system.as_deref(),
                        )?;

                        return finish_validate_code_response(
                            found,
                            &req.code,
                            &url,
                            req.display.as_deref(),
                        );
                    }
                    Err(e) => return Err(e),
                };

            // Search the expansion for the requested code.
            // When `system` is provided, match on both system + code.
            // When `system` is absent, match on code alone (first hit).
            let found = if let Some(system) = req.system.as_deref() {
                all_codes
                    .iter()
                    .find(|c| c.system == system && c.code == req.code)
                    .cloned()
            } else {
                all_codes.iter().find(|c| c.code == req.code).cloned()
            };

            finish_validate_code_response(found, &req.code, &url, req.display.as_deref())
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    /// Search ValueSet resources by query parameters.
    async fn search(
        &self,
        _ctx: &TenantContext,
        query: ResourceSearchQuery,
    ) -> Result<Vec<serde_json::Value>, HtsError> {
        let pool = self.pool().clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            let limit = i64::from(query.count.unwrap_or(20));
            let offset = i64::from(query.offset.unwrap_or(0));

            let mut stmt = conn
                .prepare_cached(
                    "SELECT id, url, version, name, title, status, resource_json
                     FROM value_sets
                     WHERE (?1 IS NULL OR url = ?1)
                       AND (?2 IS NULL OR version = ?2)
                       AND (?3 IS NULL OR name = ?3)
                       AND (?4 IS NULL OR title = ?4)
                       AND (?5 IS NULL OR status = ?5)
                     ORDER BY created_at
                     LIMIT ?6 OFFSET ?7",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            let rows = stmt
                .query_map(
                    rusqlite::params![
                        query.url,
                        query.version,
                        query.name,
                        query.title,
                        query.status,
                        limit,
                        offset
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, Option<String>>(6)?,
                        ))
                    },
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            let mut results = Vec::new();
            for row in rows {
                let (id, url, version, name, title, status, resource_json) =
                    row.map_err(|e| HtsError::StorageError(e.to_string()))?;

                let resource = resource_json
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_else(|| {
                        super::code_system::build_synthetic_resource(
                            "ValueSet",
                            &id,
                            &url,
                            version.as_deref(),
                            name.as_deref(),
                            title.as_deref(),
                            &status,
                        )
                    });
                results.push(resource);
            }
            Ok(results)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Resolve a value set by canonical URL and optional point-in-time date.
///
/// Returns `(id, compose_json)`.
/// Returns [`HtsError::NotFound`] when the URL is not in the `value_sets` table.
///
/// When `date` is provided, only value sets whose `$.date` (from `resource_json`)
/// is ≤ the requested date are matched.
fn resolve_value_set(
    conn: &Connection,
    url: &str,
    date: Option<&str>,
) -> Result<(String, Option<String>), HtsError> {
    conn.query_row(
        "SELECT id, compose_json FROM value_sets \
         WHERE url = ?1 \
           AND (?2 IS NULL OR json_extract(resource_json, '$.date') <= ?2)",
        rusqlite::params![url, date],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            HtsError::NotFound(format!("ValueSet not found: {url}"))
        }
        other => HtsError::StorageError(other.to_string()),
    })
}

/// Fetch all cached expansion entries for `vs_id`.
///
/// Returns an empty vec when no cached entries exist (cache miss).
fn fetch_cache(conn: &Connection, vs_id: &str) -> Result<Vec<ExpansionContains>, HtsError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT system_url, code, display
             FROM value_set_expansions
             WHERE value_set_id = ?1
             ORDER BY system_url, code",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    stmt.query_map([vs_id], |row| {
        Ok(ExpansionContains {
            system: row.get(0)?,
            code: row.get(1)?,
            display: row.get(2)?,
            inactive: None,
            contains: vec![],
        })
    })
    .map_err(|e| HtsError::StorageError(e.to_string()))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Expand an inline ValueSet compose with a text filter pushed down to SQL.
///
/// Called instead of `compute_expansion` when the request carries a `filter`
/// parameter and the compose is provided inline (not by URL). For each include
/// clause the filter is applied in the database rather than loading all concepts
/// into memory first — critical for full-system includes over large code systems
/// such as SNOMED CT, LOINC, or RxNorm (EX07: multi-system text filter).
///
/// Include clauses that carry compose `filter[]` entries (ECL / is-a) are
/// evaluated by `apply_compose_filters` and the text filter is then applied in
/// Rust over the (already bounded) result set.  Explicit `concept[]` lists are
/// also filtered in Rust since they are already small.
fn expand_inline_filtered(
    conn: &Connection,
    compose: &serde_json::Value,
    text_filter: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let empty_arr = vec![];
    let includes = compose["include"].as_array().unwrap_or(&empty_arr);
    let filter_lower = text_filter.to_lowercase();
    let sql_pat = format!("%{filter_lower}%");
    let mut results: Vec<ExpansionContains> = Vec::new();

    for inc in includes {
        let system_url = match inc["system"].as_str() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        let system_id: Option<String> = conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = ?1",
                [system_url],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let system_id = match system_id {
            Some(id) => id,
            None => {
                let msg = format!(
                    "CodeSystem {system_url} was not found and has been excluded from the expansion"
                );
                tracing::warn!(%system_url, "{msg}");
                warnings.push(msg);
                continue;
            }
        };

        if let Some(filter_result) = apply_compose_filters(conn, system_url, &system_id, inc)? {
            // Compose filters (ECL/is-a) already bounded the result — apply text filter in Rust.
            results.extend(filter_result.into_iter().filter(|c| {
                c.code.to_lowercase().contains(&filter_lower)
                    || c.display
                        .as_deref()
                        .map(|d| d.to_lowercase().contains(&filter_lower))
                        .unwrap_or(false)
            }));
        } else if let Some(explicit_codes) = inc["concept"].as_array() {
            // Explicit code list — filter in Rust (bounded by the list length).
            let mut stmt = conn
                .prepare_cached("SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2")
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            for entry in explicit_codes {
                let code = match entry["code"].as_str() {
                    Some(c) => c,
                    None => continue,
                };
                let display: Option<String> = stmt
                    .query_row(rusqlite::params![system_id, code], |row| row.get(0))
                    .optional()
                    .map_err(|e| HtsError::StorageError(e.to_string()))?
                    .flatten();
                let matches = code.to_lowercase().contains(&filter_lower)
                    || display
                        .as_deref()
                        .map(|d| d.to_lowercase().contains(&filter_lower))
                        .unwrap_or(false);
                if matches {
                    results.push(ExpansionContains {
                        system: system_url.to_owned(),
                        code: code.to_owned(),
                        display,
                        inactive: None,
                        contains: vec![],
                    });
                }
            }
        } else {
            // Full-system include with no explicit codes.
            // For filter strings ≥ 3 chars: use the FTS5 trigram index when it is
            // already built (O(matches)), otherwise fall back to a LIKE scan
            // (O(N), ~200–500 ms for large systems) and spawn a background task to
            // build the FTS5 index so future requests use the fast path.
            // Shorter filter strings skip FTS5 because trigrams need ≥ 3 chars.
            if filter_lower.len() >= 3 {
                let fts_ready: bool = conn
                    .query_row(
                        "SELECT EXISTS(\
                             SELECT 1 FROM concepts_fts WHERE system_id = ?1 LIMIT 1)",
                        [&system_id],
                        |r| r.get(0),
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;

                if fts_ready {
                    let match_expr = fts5_quote(&filter_lower);
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT code, display FROM concepts_fts \
                             WHERE concepts_fts MATCH ?1 AND system_id = ?2 \
                             ORDER BY code",
                        )
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    let rows = stmt
                        .query_map(rusqlite::params![match_expr, system_id], |row| {
                            Ok(ExpansionContains {
                                system: system_url.to_owned(),
                                code: row.get(0)?,
                                display: row.get(1)?,
                                inactive: None,
                                contains: vec![],
                            })
                        })
                        .map_err(|e| HtsError::StorageError(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    results.extend(rows);
                } else {
                    // FTS5 not yet built — use a LIKE scan for this request.
                    // Future requests will hit the warm FTS5 index once it is
                    // built (e.g. via the `ensure_concepts_fts` startup path).
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT code, display FROM concepts \
                             WHERE system_id = ?1 \
                               AND (LOWER(code) LIKE ?2 \
                                    OR LOWER(COALESCE(display,'')) LIKE ?2) \
                             ORDER BY code",
                        )
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    let rows = stmt
                        .query_map(rusqlite::params![system_id, sql_pat], |row| {
                            Ok(ExpansionContains {
                                system: system_url.to_owned(),
                                code: row.get(0)?,
                                display: row.get(1)?,
                                inactive: None,
                                contains: vec![],
                            })
                        })
                        .map_err(|e| HtsError::StorageError(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    results.extend(rows);
                }
            } else {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT code, display FROM concepts \
                         WHERE system_id = ?1 \
                           AND (LOWER(code) LIKE ?2 OR LOWER(display) LIKE ?2) \
                         ORDER BY code",
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                let rows = stmt
                    .query_map(rusqlite::params![system_id, sql_pat], |row| {
                        Ok(ExpansionContains {
                            system: system_url.to_owned(),
                            code: row.get(0)?,
                            display: row.get(1)?,
                            inactive: None,
                            contains: vec![],
                        })
                    })
                    .map_err(|e| HtsError::StorageError(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                results.extend(rows);
            }
        }
    }

    Ok(results)
}

/// Compute an expansion from the raw `compose_json`.
///
/// Supports:
/// - `compose.include[].system` — required in each include clause.
/// - `compose.include[].concept[]` — explicit code list; when absent, all
///   codes from the referenced system are included.
/// - `compose.exclude[]` — removes specific (system, code) pairs after
///   includes are resolved.
fn compute_expansion(
    conn: &Connection,
    compose_json: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    compute_expansion_depth(conn, compose_json, warnings, 0)
}

fn compute_expansion_depth(
    conn: &Connection,
    compose_json: Option<&str>,
    warnings: &mut Vec<String>,
    depth: u8,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let Some(raw) = compose_json else {
        return Ok(vec![]);
    };

    let compose: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| HtsError::Internal(format!("Failed to parse compose_json: {e}")))?;

    let empty_arr = vec![];
    let includes = compose["include"].as_array().unwrap_or(&empty_arr);
    let mut included: Vec<ExpansionContains> = Vec::new();

    for inc in includes {
        // Handle include.valueSet references (FHIR R4 §4.8.5):
        // one ValueSet includes all concepts from another ValueSet by URL.
        // These includes have no `system` and would otherwise be silently skipped.
        if let Some(vs_refs) = inc["valueSet"].as_array() {
            if !vs_refs.is_empty() {
                if depth >= 4 {
                    warnings.push(
                        "Max ValueSet include depth (4) reached; skipping nested valueSet references"
                            .to_owned(),
                    );
                    continue;
                }
                for vs_ref in vs_refs {
                    let ref_url = match vs_ref.as_str() {
                        Some(u) => u,
                        None => continue,
                    };
                    match resolve_value_set(conn, ref_url, None) {
                        Ok((ref_vs_id, ref_compose)) => {
                            let cached = fetch_cache(conn, &ref_vs_id)?;
                            if cached.is_empty() {
                                let nested = compute_expansion_depth(
                                    conn,
                                    ref_compose.as_deref(),
                                    warnings,
                                    depth + 1,
                                )?;
                                included.extend(nested);
                            } else {
                                included.extend(cached);
                            }
                        }
                        Err(_) => {
                            warnings.push(format!(
                                "Referenced ValueSet {ref_url} not found; excluded from expansion"
                            ));
                        }
                    }
                }
                continue;
            }
        }

        let system_url = match inc["system"].as_str() {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        // Resolve code system id from the `code_systems` table.
        let system_id: Option<String> = conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = ?1",
                [system_url],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let system_id = match system_id {
            Some(id) => id,
            None => {
                let msg = format!(
                    "CodeSystem {system_url} was not found and has been excluded from the expansion"
                );
                tracing::warn!(%system_url, "{msg}");
                warnings.push(msg);
                continue;
            }
        };

        // Check for ECL / is-a filters before falling through to the explicit
        // code list or "all concepts" paths.
        if let Some(filter_result) = apply_compose_filters(conn, system_url, &system_id, inc)? {
            included.extend(filter_result);
        } else if let Some(explicit_codes) = inc["concept"].as_array() {
            // Explicit code list: batch-fetch displays for all listed codes.
            // Using prepare_cached avoids re-compiling the same SQL for each code.
            let mut stmt = conn
                .prepare_cached("SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2")
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            for entry in explicit_codes {
                let code = match entry["code"].as_str() {
                    Some(c) => c.to_owned(),
                    None => continue,
                };

                let display: Option<String> = stmt
                    .query_row(rusqlite::params![system_id, code], |row| row.get(0))
                    .optional()
                    .map_err(|e| HtsError::StorageError(e.to_string()))?
                    .flatten();

                included.push(ExpansionContains {
                    system: system_url.to_owned(),
                    code,
                    display,
                    inactive: None,
                    contains: vec![],
                });
            }
        } else {
            // No explicit codes and no filters: include ALL concepts from the
            // referenced system.
            let mut stmt = conn
                .prepare_cached(
                    "SELECT code, display FROM concepts WHERE system_id = ?1 ORDER BY code",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            let rows = stmt
                .query_map([&system_id], |row| {
                    Ok(ExpansionContains {
                        system: system_url.to_owned(),
                        code: row.get(0)?,
                        display: row.get(1)?,
                        inactive: None,
                        contains: vec![],
                    })
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            included.extend(rows);
        }
    }

    // Apply excludes: build a (system, code) deny-set and filter.
    let excludes = compose["exclude"].as_array().unwrap_or(&empty_arr);
    let mut denied: HashSet<(String, String)> = HashSet::new();

    for exc in excludes {
        let exc_system = exc["system"].as_str().unwrap_or("").to_owned();
        if let Some(codes) = exc["concept"].as_array() {
            for entry in codes {
                if let Some(code) = entry["code"].as_str() {
                    denied.insert((exc_system.clone(), code.to_owned()));
                }
            }
        }
    }

    if !denied.is_empty() {
        included.retain(|c| !denied.contains(&(c.system.clone(), c.code.clone())));
    }

    Ok(included)
}

/// Evaluate any ECL or `is-a` filters declared on a compose include clause.
///
/// When a `compose.include[]` entry carries a `filter` array, this function
/// evaluates every entry in that array and returns the resulting concept set.
/// Multiple filters on the same include clause are **intersected** (AND
/// semantics), matching the behaviour described in FHIR R5 §4.9.5.
///
/// # Return value
///
/// | Case | Return |
/// |------|--------|
/// | No `filter` key, or `filter` is an empty array | `Ok(None)` — caller should use the normal code-list / all-concepts path |
/// | At least one recognised filter evaluated successfully | `Ok(Some(concepts))` |
/// | All filter entries have an unrecognised `property`/`op` | `Ok(Some([]))` — an empty expansion (not all concepts) |
/// | A recognised filter fails to parse or evaluate | `Err(_)` |
///
/// # Recognised filters
///
/// | `property`    | `op`          | Interpretation |
/// |---------------|---------------|----------------|
/// | `constraint`  | `=`           | Full ECL expression (e.g. `<< 404684003`) |
/// | `concept`     | `is-a`        | Subsumption — translated to `<< <value>` (descendants + self) |
/// | `concept`     | `descendent-of` | Strict subsumption — translated to `< <value>` (descendants only) |
/// | `concept`     | `generalizes`   | Ancestors-of — translated to `>> <value>` (self + ancestors) |
/// | _any other_   | `=`           | Property equality — queries `concept_properties` table |
///
/// Unrecognised `(property, op)` pairs emit a `WARN` trace event and are
/// treated as yielding an empty set so they do not silently expand the whole
/// code system.
///
/// # Filter ordering optimisation
///
/// Property equality filters (small, indexed) are evaluated first regardless
/// of their position in the array.  When a bounded candidate set is available
/// from those filters, any subsequent hierarchy filter (`is-a`, `descendent-of`,
/// `generalizes`) checks membership by walking **up** from each candidate
/// (O(depth × N_candidates)) rather than expanding the full subtree downward
/// (O(N_descendants)).  For large hierarchies such as SNOMED CT this can reduce
/// work from O(350 000) to O(50 × 15).
fn apply_compose_filters(
    conn: &Connection,
    system_url: &str,
    system_id: &str,
    inc: &serde_json::Value,
) -> Result<Option<Vec<ExpansionContains>>, HtsError> {
    let filters = match inc["filter"].as_array() {
        Some(f) if !f.is_empty() => f,
        _ => return Ok(None),
    };

    // Partition into property= filters (fast, indexed) and hierarchy filters
    // (potentially O(N_descendants)).  Property filters run in phase 1; hierarchy
    // filters run in phase 2 and can exploit the bounded candidate set from
    // phase 1 to switch from a top-down tree expansion to per-candidate ancestor
    // walks.
    let (property_filters, hierarchy_filters): (Vec<_>, Vec<_>) = filters.iter().partition(|f| {
        let op = f["op"].as_str().unwrap_or("");
        let property = f["property"].as_str().unwrap_or("");
        op == "=" && property != "constraint"
    });

    let mut result: Option<Vec<ExpansionContains>> = None;
    let mut any_filter_seen = false;

    // ── Phase 1: property equality filters ────────────────────────────────────
    for f in &property_filters {
        let property = f["property"].as_str().unwrap_or("");
        let value = f["value"].as_str().unwrap_or("");
        any_filter_seen = true;
        let concepts = query_property_eq(conn, system_url, system_id, property, value)?;
        match result.as_mut() {
            Some(prev) => {
                let keep: HashSet<String> = concepts.iter().map(|c| c.code.clone()).collect();
                prev.retain(|c| keep.contains(&c.code));
            }
            None => result = Some(concepts),
        }
    }

    // ── Phase 2: ECL / hierarchy filters ──────────────────────────────────────
    for f in &hierarchy_filters {
        let property = f["property"].as_str().unwrap_or("");
        let op = f["op"].as_str().unwrap_or("");
        let value = f["value"].as_str().unwrap_or("");

        let ecl_expr: String = match (property, op) {
            ("constraint", "=") => value.to_owned(),
            ("concept", "is-a") => format!("<< {value}"),
            ("concept", "descendent-of") => format!("< {value}"),
            // generalizes: all X such that value is-a X (ancestors of value + self).
            ("concept", "generalizes") => format!(">> {value}"),
            _ => {
                tracing::warn!(
                    property,
                    op,
                    "Unsupported compose filter — treating as empty set"
                );
                any_filter_seen = true;
                result = Some(vec![]);
                continue;
            }
        };

        any_filter_seen = true;

        // Fast path: a bounded candidate set from phase 1 exists — check
        // hierarchy membership per concept by walking UP instead of expanding
        // the whole subtree DOWN.  Skip the fast path when the candidate set is
        // already empty (intersection is trivially empty).
        if let Some(prev) = result.as_mut() {
            if !prev.is_empty() {
                match (property, op) {
                    ("concept", "is-a") => {
                        prev.retain(|c| {
                            check_is_descendant_of(conn, system_id, &c.code, value, true)
                                .unwrap_or(false)
                        });
                        continue;
                    }
                    ("concept", "descendent-of") => {
                        prev.retain(|c| {
                            check_is_descendant_of(conn, system_id, &c.code, value, false)
                                .unwrap_or(false)
                        });
                        continue;
                    }
                    ("concept", "generalizes") => {
                        // C generalizes value  ⟺  value is-a C  ⟺  C is an ancestor of value.
                        // Equivalent to: value is a descendant-or-self of C.
                        prev.retain(|c| {
                            check_is_descendant_of(conn, system_id, value, &c.code, true)
                                .unwrap_or(false)
                        });
                        continue;
                    }
                    _ => {}
                }
            }
        }

        // Slow path: no prior bounded set — compute the full ECL expansion.
        let resolved = ecl::parse_and_evaluate(conn, system_id, &ecl_expr)?;
        let concepts: Vec<ExpansionContains> = resolved
            .into_iter()
            .map(|c| ExpansionContains {
                system: system_url.to_owned(),
                code: c.code,
                display: c.display,
                inactive: None,
                contains: vec![],
            })
            .collect();

        match result.as_mut() {
            Some(prev) => {
                let keep: HashSet<String> = concepts.iter().map(|c| c.code.clone()).collect();
                prev.retain(|c| keep.contains(&c.code));
            }
            None => result = Some(concepts),
        }
    }

    if any_filter_seen && result.is_none() {
        return Ok(Some(vec![]));
    }

    Ok(result)
}

/// Look up all concepts in `system_id` that have a property matching
/// `(property = value)` in the `concept_properties` table.
fn query_property_eq(
    conn: &Connection,
    system_url: &str,
    system_id: &str,
    property: &str,
    value: &str,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.code, c.display
             FROM concepts c
             JOIN concept_properties cp ON cp.concept_id = c.id
             WHERE c.system_id = ?1
               AND cp.property = ?2
               AND cp.value = ?3",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows = stmt
        .query_map([system_id, property, value], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|(code, display)| ExpansionContains {
            system: system_url.to_owned(),
            code,
            display,
            inactive: None,
            contains: vec![],
        })
        .collect())
}

/// Check whether `candidate_code` is a descendant-or-self (when `include_self=true`)
/// or a strict descendant (when `include_self=false`) of `root_code`.
///
/// Walks **up** from `candidate_code` through `concept_hierarchy` in O(depth)
/// time.  Used by `apply_compose_filters` to avoid expanding the full descendant
/// subtree when a bounded candidate set is already available from a property=
/// filter.
fn check_is_descendant_of(
    conn: &Connection,
    system_id: &str,
    candidate_code: &str,
    root_code: &str,
    include_self: bool,
) -> Result<bool, HtsError> {
    if candidate_code == root_code {
        return Ok(include_self);
    }
    conn.query_row(
        "WITH RECURSIVE anc(code) AS (
             SELECT ?1
             UNION ALL
             SELECT ch.parent_code
             FROM   concept_hierarchy ch
             JOIN   anc ON ch.child_code = anc.code
             WHERE  ch.system_id = ?2
         )
         SELECT EXISTS(SELECT 1 FROM anc WHERE code = ?3)",
        rusqlite::params![candidate_code, system_id, root_code],
        |r| r.get(0),
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// FNV-1a 64-bit hash — deterministic, no external dependencies, no random seed.
///
/// Used to derive stable cache keys for inline compose expansions.
fn fnv64(data: &[u8]) -> u64 {
    const PRIME: u64 = 0x00000100000001B3;
    const OFFSET: u64 = 0xcbf29ce484222325;
    let mut h = OFFSET;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Pattern extracted from a `?fhir_vs` implicit ValueSet URL.
///
/// FHIR defines query-parameter patterns on a CodeSystem URL that implicitly
/// describe a ValueSet (FHIR R4 §4.8.7):
///
/// | URL form | Pattern | Meaning |
/// |---|---|---|
/// | `<cs>?fhir_vs` | `AllConcepts` | Every code in the CodeSystem |
/// | `<cs>?fhir_vs=isa/<code>` | `IsA(code)` | Descendants (subsumees) of `code` |
#[derive(Debug)]
enum FhirVsPattern {
    AllConcepts,
    IsA(String),
}

/// Parse a `?fhir_vs` implicit ValueSet URL.
///
/// Returns `Some((cs_url, pattern))` on a recognised pattern, `None` otherwise.
fn parse_fhir_vs_url(url: &str) -> Option<(String, FhirVsPattern)> {
    let (base, query) = url.split_once('?')?;
    if !query.starts_with("fhir_vs") {
        return None;
    }
    let rest = &query["fhir_vs".len()..];
    if rest.is_empty() {
        return Some((base.to_owned(), FhirVsPattern::AllConcepts));
    }
    let value = rest.strip_prefix('=')?;
    if let Some(code) = value.strip_prefix("isa/") {
        return Some((base.to_owned(), FhirVsPattern::IsA(code.to_owned())));
    }
    None
}

/// Check whether a compose is a "simple hierarchy" and extract its parameters.
///
/// Serve a paginated page from a purely extensional compose (all includes have
/// explicit `concept[]` lists, no `filter[]`).
///
/// Returns `Some(page)` when the compose is fully extensional and we can serve
/// `offset..offset+limit` codes by looking up only those rows in the database.
/// Returns `None` when any include has filters or no explicit code list, so the
/// caller falls through to the full `compute_expansion` path.
///
/// This lets large VSAC ValueSets (thousands of explicit codes spread across
/// one or more systems) serve the first page in milliseconds instead of
/// requiring a full DB scan that can exceed the 30 s request timeout.
fn compose_page_fast(
    conn: &Connection,
    compose_json: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<Option<(Vec<ExpansionContains>, u32)>, HtsError> {
    let compose: serde_json::Value = match compose_json {
        Some(s) => match serde_json::from_str(s) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        },
        None => return Ok(None),
    };

    let includes = match compose["include"].as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return Ok(None),
    };

    // Only handle purely extensional composes: every include must have concept[]
    // and no filter[].  Mixed or intensional includes fall through to slow path.
    for inc in includes {
        if inc["concept"].as_array().is_none() {
            return Ok(None);
        }
        if inc["filter"].as_array().is_some_and(|f| !f.is_empty()) {
            return Ok(None);
        }
    }

    // Collect (system_url, code) pairs in compose order.
    let mut all_pairs: Vec<(String, String)> = Vec::new();
    for inc in includes {
        let system_url = match inc["system"].as_str() {
            Some(s) if !s.is_empty() => s.to_owned(),
            _ => continue,
        };
        if let Some(concepts) = inc["concept"].as_array() {
            for c in concepts {
                if let Some(code) = c["code"].as_str() {
                    all_pairs.push((system_url.clone(), code.to_owned()));
                }
            }
        }
    }

    // Apply exclusions (purely code-based).
    let excludes = compose["exclude"].as_array();
    if let Some(excl) = excludes {
        if !excl.is_empty() {
            let mut exclude_set: HashSet<(String, String)> = HashSet::new();
            for exc in excl {
                let sys = exc["system"].as_str().unwrap_or("").to_owned();
                if let Some(concepts) = exc["concept"].as_array() {
                    for c in concepts {
                        if let Some(code) = c["code"].as_str() {
                            exclude_set.insert((sys.clone(), code.to_owned()));
                        }
                    }
                }
            }
            all_pairs.retain(|p| !exclude_set.contains(p));
        }
    }

    let total = all_pairs.len() as u32;

    // Paginate: take only the slice we need.
    let page_pairs: Vec<(String, String)> =
        all_pairs.into_iter().skip(offset).take(limit).collect();

    if page_pairs.is_empty() {
        return Ok(Some((vec![], total)));
    }

    // Look up displays for only the page slice — O(limit) queries.
    let mut result = Vec::with_capacity(page_pairs.len());
    let mut system_cache: HashMap<String, Option<String>> = HashMap::new();

    for (system_url, code) in &page_pairs {
        let system_id: Option<String> = system_cache
            .entry(system_url.clone())
            .or_insert_with(|| {
                conn.query_row(
                    "SELECT id FROM code_systems WHERE url = ?1",
                    [system_url.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .ok()
                .flatten()
            })
            .clone();

        let display: Option<String> = if let Some(sid) = system_id {
            conn.query_row(
                "SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2",
                rusqlite::params![sid, code],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .flatten()
        } else {
            None
        };

        result.push(ExpansionContains {
            system: system_url.clone(),
            code: code.clone(),
            display,
            inactive: None,
            contains: vec![],
        });
    }

    Ok(Some((result, total)))
}

/// Matches composes with exactly one include clause that carries exactly one
/// filter of type `concept is-a` or `concept descendent-of`.  Richer composes
/// (multi-filter, property= filters, multiple includes) fall through to the
/// slow blocking path so they benefit from caching on second call.
///
/// Returns `Some((system_url, system_id, root_code, include_root))` on a match,
/// `None` when the compose does not fit the pattern.
fn extract_simple_hierarchy_compose(
    conn: &Connection,
    compose: &serde_json::Value,
    warnings: &mut Vec<String>,
) -> Result<Option<(String, String, String, bool)>, HtsError> {
    let includes = match compose["include"].as_array() {
        Some(a) if a.len() == 1 => a,
        _ => return Ok(None),
    };
    let inc = &includes[0];

    let filters = match inc["filter"].as_array() {
        Some(f) if f.len() == 1 => f,
        _ => return Ok(None),
    };
    let f = &filters[0];

    let property = f["property"].as_str().unwrap_or("");
    let op = f["op"].as_str().unwrap_or("");
    let root_code = f["value"].as_str().unwrap_or("");

    if property != "concept" || root_code.is_empty() {
        return Ok(None);
    }

    let include_root = match op {
        "is-a" => true,
        "descendent-of" => false,
        _ => return Ok(None),
    };

    let system_url = match inc["system"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(None),
    };

    let system_id: Option<String> = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            [system_url],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let system_id = match system_id {
        Some(id) => id,
        None => {
            warnings.push(format!(
                "CodeSystem {system_url} was not found and has been excluded from the expansion"
            ));
            return Ok(None);
        }
    };

    Ok(Some((
        system_url.to_owned(),
        system_id,
        root_code.to_owned(),
        include_root,
    )))
}

/// Serve a page of an implicit ValueSet without waiting for the full cache.
///
/// Used as the "cold-cache fast path" when `ensure_implicit_cache` would block
/// for >30 s (e.g. SNOMED CT `?fhir_vs=isa/404684003` with ~350 K descendants).
///
/// - `AllConcepts`: direct indexed SQL `LIMIT/OFFSET` — O(log N).
/// - `IsA`: BFS from the root, stopping after `offset + limit` nodes — O(offset+limit).
fn bfs_expand_page(
    conn: &Connection,
    cs_url: &str,
    system_id: &str,
    pattern: &FhirVsPattern,
    offset: usize,
    limit: usize,
    filter_lower: Option<&str>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    match pattern {
        FhirVsPattern::AllConcepts => {
            let sql_limit = limit as i64;
            let sql_offset = offset as i64;
            if let Some(f) = filter_lower {
                let sql_pat = format!("%{f}%");
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT code, display FROM concepts \
                         WHERE system_id = ?1 \
                           AND (LOWER(code) LIKE ?2 \
                                OR LOWER(COALESCE(display,'')) LIKE ?2) \
                         ORDER BY code LIMIT ?3 OFFSET ?4",
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                stmt.query_map(
                    rusqlite::params![system_id, sql_pat, sql_limit, sql_offset],
                    |r| {
                        Ok(ExpansionContains {
                            system: cs_url.to_owned(),
                            code: r.get(0)?,
                            display: r.get(1)?,
                            inactive: None,
                            contains: vec![],
                        })
                    },
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HtsError::StorageError(e.to_string()))
            } else {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT code, display FROM concepts \
                         WHERE system_id = ?1 ORDER BY code LIMIT ?2 OFFSET ?3",
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                stmt.query_map(rusqlite::params![system_id, sql_limit, sql_offset], |r| {
                    Ok(ExpansionContains {
                        system: cs_url.to_owned(),
                        code: r.get(0)?,
                        display: r.get(1)?,
                        inactive: None,
                        contains: vec![],
                    })
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HtsError::StorageError(e.to_string()))
            }
        }
        FhirVsPattern::IsA(root_code) => bfs_isa_page(
            conn,
            cs_url,
            system_id,
            root_code,
            true, // ?fhir_vs=isa/X is self + descendants (<< semantics)
            offset,
            limit,
            filter_lower,
        ),
    }
}

/// BFS traversal of an `IsA` or `DescendentOf` hierarchy, returning one page.
///
/// Visits nodes breadth-first, collecting those that pass the optional text
/// filter, skipping the first `offset` and returning up to `limit` more.
/// Each node costs two indexed SQL lookups (display + children), so visiting
/// `offset + limit` nodes runs in O((offset+limit) × log N) — typically a
/// few hundred milliseconds even at offset=500, versus 30+ seconds for the
/// full recursive-CTE INSERT.
///
/// `include_root=true` adds `root_code` itself to the result set (is-a /
/// `<<` semantics); `false` starts BFS from root's children (descendent-of /
/// `<` semantics).
#[allow(clippy::too_many_arguments)]
fn bfs_isa_page(
    conn: &Connection,
    cs_url: &str,
    system_id: &str,
    root_code: &str,
    include_root: bool,
    offset: usize,
    limit: usize,
    filter_lower: Option<&str>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(root_code.to_owned());

    if include_root {
        queue.push_back(root_code.to_owned());
    } else {
        // descendent-of: seed the queue with root's direct children
        let mut stmt = conn
            .prepare_cached(
                "SELECT child_code FROM concept_hierarchy \
                 WHERE system_id = ?1 AND parent_code = ?2 ORDER BY child_code",
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let children: Vec<String> = stmt
            .query_map(rusqlite::params![system_id, root_code], |r| r.get(0))
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        for child in children {
            if visited.insert(child.clone()) {
                queue.push_back(child);
            }
        }
    }

    let mut visible: usize = 0; // count of nodes that passed the filter so far
    let mut page: Vec<ExpansionContains> = Vec::new();

    while let Some(code) = queue.pop_front() {
        let display: Option<String> = conn
            .query_row(
                "SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2",
                rusqlite::params![system_id, code],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .flatten();

        let passes = match filter_lower {
            Some(f) => {
                code.to_lowercase().contains(f)
                    || display
                        .as_deref()
                        .map(|d| d.to_lowercase().contains(f))
                        .unwrap_or(false)
            }
            None => true,
        };

        if passes {
            if visible >= offset && page.len() < limit {
                page.push(ExpansionContains {
                    system: cs_url.to_owned(),
                    code: code.clone(),
                    display,
                    inactive: None,
                    contains: vec![],
                });
                if page.len() >= limit {
                    break;
                }
            }
            visible += 1;
        }

        // Fetch children (indexed lookup on PRIMARY KEY)
        let mut stmt = conn
            .prepare_cached(
                "SELECT child_code FROM concept_hierarchy \
                 WHERE system_id = ?1 AND parent_code = ?2 ORDER BY child_code",
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let children: Vec<String> = stmt
            .query_map(rusqlite::params![system_id, code], |r| r.get(0))
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        for child in children {
            if visited.insert(child.clone()) {
                queue.push_back(child);
            }
        }
    }

    Ok(page)
}

/// Find the canonical URL of a CodeSystem whose `valueSet` property equals `vs_url`.
///
/// When a CodeSystem carries `"valueSet": "http://..."` it implicitly defines a
/// ValueSet containing all its codes.  This function resolves that link so
/// `$expand` can fall back to an implicit expansion when no explicit ValueSet
/// resource exists for the requested URL.
///
/// Returns [`HtsError::NotFound`] when no matching CodeSystem is found.
fn find_cs_for_implicit_vs(
    conn: &Connection,
    vs_url: &str,
    date: Option<&str>,
) -> Result<String, HtsError> {
    conn.query_row(
        "SELECT url FROM code_systems \
         WHERE json_extract(resource_json, '$.valueSet') = ?1 \
           AND (?2 IS NULL OR json_extract(resource_json, '$.date') <= ?2)",
        rusqlite::params![vs_url, date],
        |row| row.get::<_, String>(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            HtsError::NotFound(format!("ValueSet not found: {vs_url}"))
        }
        other => HtsError::StorageError(other.to_string()),
    })
}

/// Build a tree-structured expansion from a flat list of concepts.
///
/// Uses the `concept_hierarchy` table to determine parent-child relationships.
/// Only edges where **both** parent and child appear in the flat expansion are
/// used — orphaned codes (whose parent is not in the expansion) become roots.
///
/// The returned list contains only root-level concepts; children are nested in
/// each `ExpansionContains::contains` field recursively.
fn build_hierarchical_expansion(
    conn: &Connection,
    flat: Vec<ExpansionContains>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    if flat.is_empty() {
        return Ok(flat);
    }

    // Build lookup: (system_url, code) → ExpansionContains.
    let items_map: HashMap<(String, String), ExpansionContains> = flat
        .iter()
        .cloned()
        .map(|c| ((c.system.clone(), c.code.clone()), c))
        .collect();

    // Set of all (system_url, code) pairs in the expansion for fast membership checks.
    let expansion_set: HashSet<(String, String)> = flat
        .iter()
        .map(|c| (c.system.clone(), c.code.clone()))
        .collect();

    // For each unique system URL, look up the system_id from code_systems.
    let system_urls: HashSet<String> = flat.iter().map(|c| c.system.clone()).collect();
    let mut system_id_map: HashMap<String, String> = HashMap::new();
    for sys_url in &system_urls {
        if let Some(id) = conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = ?1",
                [sys_url],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| HtsError::StorageError(e.to_string()))?
        {
            system_id_map.insert(sys_url.clone(), id);
        }
    }

    // For each system, query all parent-child edges; keep only those where
    // both endpoints are in the expansion.
    // parent_to_children: (system_url, parent_code) → Vec<(system_url, child_code)>
    let mut parent_to_children: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    // has_parent: tracks which codes have a parent within the expansion.
    let mut has_parent: HashSet<(String, String)> = HashSet::new();

    for (sys_url, sys_id) in &system_id_map {
        let mut stmt = conn
            .prepare_cached(
                "SELECT parent_code, child_code
                 FROM concept_hierarchy
                 WHERE system_id = ?1",
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let edges: Vec<(String, String)> = stmt
            .query_map([sys_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<Result<_, _>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        for (parent_code, child_code) in edges {
            let parent_key = (sys_url.clone(), parent_code);
            let child_key = (sys_url.clone(), child_code);
            if expansion_set.contains(&parent_key) && expansion_set.contains(&child_key) {
                parent_to_children
                    .entry(parent_key)
                    .or_default()
                    .push(child_key.clone());
                has_parent.insert(child_key);
            }
        }
    }

    // Roots: concepts that appear in the expansion but have no parent within it.
    let mut roots: Vec<ExpansionContains> = flat
        .iter()
        .filter(|c| !has_parent.contains(&(c.system.clone(), c.code.clone())))
        .map(|c| {
            build_subtree(
                &(c.system.clone(), c.code.clone()),
                &items_map,
                &parent_to_children,
            )
        })
        .collect();

    roots.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(roots)
}

/// Recursively build an [`ExpansionContains`] node with all its nested children.
///
/// Looks up `key` in `items_map` to get the base node, then checks
/// `parent_to_children` for any children of that node, recursing into each
/// child.  Children are sorted by code before being attached, producing a
/// deterministic tree order regardless of the order edges were stored in
/// `concept_hierarchy`.
///
/// ## Parameters
/// - `key` — `(system_url, code)` of the concept to build.
/// - `items_map` — flat `(system_url, code)` → [`ExpansionContains`] lookup.
/// - `parent_to_children` — adjacency map built from `concept_hierarchy` edges
///   that are fully contained within the expansion set.
fn build_subtree(
    key: &(String, String),
    items_map: &HashMap<(String, String), ExpansionContains>,
    parent_to_children: &HashMap<(String, String), Vec<(String, String)>>,
) -> ExpansionContains {
    let mut item = items_map[key].clone();
    if let Some(children) = parent_to_children.get(key) {
        let mut child_items: Vec<ExpansionContains> = children
            .iter()
            .map(|ck| build_subtree(ck, items_map, parent_to_children))
            .collect();
        child_items.sort_by(|a, b| a.code.cmp(&b.code));
        item.contains = child_items;
    }
    item
}

/// Write computed expansion entries into the `value_set_expansions` cache.
///
/// Any existing entries for `vs_id` are deleted first so re-computation
/// (e.g. after a ValueSet update) always produces a clean cache.
/// All inserts are wrapped in a single transaction for performance — without
/// an explicit transaction, SQLite auto-commits each row individually, which
/// for large ValueSets (e.g. 6000+ VSAC concepts) can easily exceed the
/// 30-second request timeout.
fn populate_cache(
    conn: &Connection,
    vs_id: &str,
    codes: &[ExpansionContains],
) -> Result<(), HtsError> {
    // BEGIN IMMEDIATE acquires the write lock upfront so concurrent callers
    // cannot both see an empty cache and then duplicate-write the expansion.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Re-check inside the lock: another VU may have populated this while we
    // were waiting to acquire the write lock.
    let already: bool = match conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM value_set_expansions WHERE value_set_id = ?1 LIMIT 1)",
        [vs_id],
        |r| r.get(0),
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
    };

    if already {
        conn.execute_batch("COMMIT")
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        return Ok(());
    }

    if let Err(e) = conn.execute(
        "DELETE FROM value_set_expansions WHERE value_set_id = ?1",
        [vs_id],
    ) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(HtsError::StorageError(e.to_string()));
    }

    {
        let mut stmt = match conn.prepare_cached(
            "INSERT OR IGNORE INTO value_set_expansions
             (value_set_id, system_url, code, display)
             VALUES (?1, ?2, ?3, ?4)",
        ) {
            Ok(s) => s,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(HtsError::StorageError(e.to_string()));
            }
        };
        for item in codes {
            if let Err(e) = stmt.execute(rusqlite::params![
                vs_id,
                item.system,
                item.code,
                item.display
            ]) {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(HtsError::StorageError(e.to_string()));
            }
        }
    }

    conn.execute_batch("COMMIT")
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Build a [`ValidateCodeResponse`] from an optional matching concept.
///
/// Shared by all validate-code paths (explicit ValueSet, implicit cache, and
/// direct `?fhir_vs` lookups) so display-mismatch logic is applied consistently.
fn finish_validate_code_response(
    found: Option<ExpansionContains>,
    code: &str,
    url: &str,
    expected_display: Option<&str>,
) -> Result<ValidateCodeResponse, HtsError> {
    match found {
        None => Ok(ValidateCodeResponse {
            result: false,
            message: Some(format!("Code '{code}' is not in value set '{url}'")),
            display: None,
        }),
        Some(concept) => {
            let mut message = None;
            if let Some(expected) = expected_display {
                if let Some(actual) = concept.display.as_deref() {
                    if !actual.eq_ignore_ascii_case(expected) {
                        message = Some(format!(
                            "Provided display '{expected}' does not match stored display '{actual}'"
                        ));
                    }
                }
            }
            Ok(ValidateCodeResponse {
                result: message.is_none(),
                message,
                display: concept.display,
            })
        }
    }
}

/// Validate a code against a `?fhir_vs` implicit ValueSet pattern directly,
/// without materializing the full expansion into the cache.
///
/// - `AllConcepts` — O(1) point lookup in the `concepts` table.
/// - `IsA(root)` — O(depth) recursive CTE walking *up* from `code` through
///   `concept_hierarchy` to check whether `root` is an ancestor-or-self.
///
/// Returns the matching [`ExpansionContains`] on success, or `None` when the
/// code is not a member of the implicit ValueSet.
fn validate_fhir_vs(
    conn: &Connection,
    cs_url: &str,
    pattern: &FhirVsPattern,
    code: &str,
    system: Option<&str>,
) -> Result<Option<ExpansionContains>, HtsError> {
    // If system is provided it must match the CodeSystem URL.
    if let Some(sys) = system {
        if sys != cs_url {
            return Ok(None);
        }
    }

    let system_id: Option<String> = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            [cs_url],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let system_id =
        system_id.ok_or_else(|| HtsError::NotFound(format!("CodeSystem not found: {cs_url}")))?;

    match pattern {
        FhirVsPattern::AllConcepts => {
            let row = conn
                .query_row(
                    "SELECT code, display FROM concepts WHERE system_id = ?1 AND code = ?2",
                    rusqlite::params![system_id, code],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                )
                .optional()
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            Ok(row.map(|(code, display)| ExpansionContains {
                system: cs_url.to_owned(),
                code,
                display,
                inactive: None,
                contains: vec![],
            }))
        }
        FhirVsPattern::IsA(root_code) => {
            // Walk UP from `code` through concept_hierarchy to find whether
            // root_code is an ancestor-or-self. O(depth), not O(tree size).
            let is_member: bool = conn
                .query_row(
                    "WITH RECURSIVE ancestors(code) AS (
                         SELECT ?1
                         UNION ALL
                         SELECT ch.parent_code
                         FROM concept_hierarchy ch
                         JOIN ancestors a ON ch.child_code = a.code
                         WHERE ch.system_id = ?2
                     )
                     SELECT EXISTS(SELECT 1 FROM ancestors WHERE code = ?3)",
                    rusqlite::params![code, system_id, root_code],
                    |r| r.get(0),
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            if !is_member {
                return Ok(None);
            }

            let display: Option<String> = conn
                .query_row(
                    "SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2",
                    rusqlite::params![system_id, code],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .flatten();

            Ok(Some(ExpansionContains {
                system: cs_url.to_owned(),
                code: code.to_owned(),
                display,
                inactive: None,
                contains: vec![],
            }))
        }
    }
}

/// Ensure the implicit expansion cache is populated for `url`.
///
/// If the cache already has entries the function returns immediately (fast path).
/// Otherwise, determines the backing code system and writes all matching concepts
/// atomically using `INSERT … SELECT` — avoids materialising hundreds-of-thousands
/// of rows in Rust and is typically 10–50× faster than the previous row-loop
/// approach for large systems such as SNOMED CT (~350 K concepts).
fn ensure_implicit_cache(conn: &Connection, url: &str, date: Option<&str>) -> Result<(), HtsError> {
    let populated: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM implicit_expansion_cache WHERE url = ?1 LIMIT 1)",
            [url],
            |r| r.get(0),
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if populated {
        return Ok(());
    }

    // Determine the code system and the set of concepts to cache.
    // AllConcepts is also used for the CodeSystem.valueSet link path.
    let (cs_url, pattern) = if let Ok(cs_url) = find_cs_for_implicit_vs(conn, url, date) {
        (cs_url, FhirVsPattern::AllConcepts)
    } else if let Some((cs_url, pat)) = parse_fhir_vs_url(url) {
        (cs_url, pat)
    } else {
        return Err(HtsError::NotFound(format!("ValueSet not found: {url}")));
    };

    let system_id: Option<String> = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            [&cs_url],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    let system_id =
        system_id.ok_or_else(|| HtsError::NotFound(format!("CodeSystem not found: {cs_url}")))?;

    // BEGIN IMMEDIATE acquires the write lock upfront so concurrent callers
    // cannot both see an empty cache and then duplicate-write the expansion.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Re-check inside the lock: another VU may have populated this while we
    // were waiting to acquire the write lock.
    let still_empty: bool = match conn.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM implicit_expansion_cache WHERE url = ?1 LIMIT 1)",
        [url],
        |r| r.get(0),
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
    };

    if !still_empty {
        conn.execute_batch("COMMIT")
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        return Ok(());
    }

    if let Err(e) = conn.execute("DELETE FROM implicit_expansion_cache WHERE url = ?1", [url]) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(HtsError::StorageError(e.to_string()));
    }

    let insert_result = match &pattern {
        FhirVsPattern::AllConcepts => conn.execute(
            "INSERT OR IGNORE INTO implicit_expansion_cache (url, system_url, code, display)
             SELECT ?1, ?2, code, display FROM concepts WHERE system_id = ?3",
            rusqlite::params![url, cs_url, system_id],
        ),
        FhirVsPattern::IsA(root_code) => {
            // Recursive CTE expands the descendant subtree directly in SQL.
            // The seed row is the root itself (<< semantics: self + descendants).
            conn.execute(
                "INSERT OR IGNORE INTO implicit_expansion_cache (url, system_url, code, display)
                 WITH RECURSIVE desc_cte(code) AS (
                     SELECT ?4
                     UNION ALL
                     SELECT h.child_code
                     FROM   concept_hierarchy h
                     JOIN   desc_cte d ON h.parent_code = d.code
                     WHERE  h.system_id = ?3
                 )
                 SELECT ?1, ?2, c.code, c.display
                 FROM   concepts c
                 JOIN   desc_cte d ON c.code = d.code
                 WHERE  c.system_id = ?3",
                rusqlite::params![url, cs_url, system_id, root_code],
            )
        }
    };

    if let Err(e) = insert_result {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(HtsError::StorageError(e.to_string()));
    }

    conn.execute_batch("COMMIT")
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Look up a single code in the implicit expansion cache.
///
/// Returns the matching `ExpansionContains` when found, or `None` on a miss.
fn lookup_in_implicit_cache(
    conn: &Connection,
    url: &str,
    code: &str,
    system: Option<&str>,
) -> Result<Option<ExpansionContains>, HtsError> {
    let row = if let Some(sys) = system {
        conn.query_row(
            "SELECT system_url, code, display
             FROM implicit_expansion_cache
             WHERE url = ?1 AND code = ?2 AND system_url = ?3
             LIMIT 1",
            rusqlite::params![url, code, sys],
            |r| {
                Ok(ExpansionContains {
                    system: r.get(0)?,
                    code: r.get(1)?,
                    display: r.get(2)?,
                    inactive: None,
                    contains: vec![],
                })
            },
        )
    } else {
        conn.query_row(
            "SELECT system_url, code, display
             FROM implicit_expansion_cache
             WHERE url = ?1 AND code = ?2
             LIMIT 1",
            rusqlite::params![url, code],
            |r| {
                Ok(ExpansionContains {
                    system: r.get(0)?,
                    code: r.get(1)?,
                    display: r.get(2)?,
                    inactive: None,
                    contains: vec![],
                })
            },
        )
    };

    match row {
        Ok(c) => Ok(Some(c)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(HtsError::StorageError(e.to_string())),
    }
}

/// Wrap a search term as an FTS5 phrase literal.
///
/// Double-quotes the term so FTS5 treats it as a substring phrase rather than
/// individual tokens.  Internal double-quote characters are escaped by doubling.
fn fts5_quote(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Count cached entries matching an optional filter for an implicit VS URL.
///
/// Ensure the FTS5 mirror of the implicit expansion cache is populated for `url`.
///
/// Populated lazily — only called when a text filter is actually needed so that
/// unfiltered requests (e.g. EX01 hierarchy expansions) pay no FTS5 overhead.
/// Reads rows from `implicit_expansion_cache` and bulk-inserts them into
/// `implicit_expansion_fts` via a single `INSERT … SELECT` statement.
fn ensure_implicit_fts(conn: &Connection, url: &str) -> Result<(), HtsError> {
    let populated: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM implicit_expansion_fts WHERE url = ?1 LIMIT 1)",
            [url],
            |r| r.get(0),
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if populated {
        return Ok(());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    tx.execute("DELETE FROM implicit_expansion_fts WHERE url = ?1", [url])
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    tx.execute(
        "INSERT INTO implicit_expansion_fts (url, system_url, code, display)
         SELECT url, system_url, code, display
         FROM implicit_expansion_cache
         WHERE url = ?1",
        [url],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    tx.commit()
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Ensure the FTS5 trigram index on `concepts_fts` is populated for `system_id`.
///
/// Populated lazily on the first filtered inline expand for a given system.
/// Cleared on server startup so a re-import followed by a restart always
/// rebuilds from fresh data.
#[allow(dead_code)]
fn ensure_concepts_fts(conn: &Connection, system_id: &str) -> Result<(), HtsError> {
    let populated: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM concepts_fts WHERE system_id = ?1 LIMIT 1)",
            [system_id],
            |r| r.get(0),
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if populated {
        return Ok(());
    }

    // BEGIN IMMEDIATE acquires the write lock upfront so concurrent background
    // tasks don't each build the same index independently.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Re-check inside the lock: another task may have built the index while we waited.
    let still_empty: bool = match conn.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM concepts_fts WHERE system_id = ?1 LIMIT 1)",
        [system_id],
        |r| r.get(0),
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
    };

    if !still_empty {
        conn.execute_batch("COMMIT")
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        return Ok(());
    }

    if let Err(e) = conn.execute("DELETE FROM concepts_fts WHERE system_id = ?1", [system_id]) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(HtsError::StorageError(e.to_string()));
    }

    if let Err(e) = conn.execute(
        "INSERT INTO concepts_fts(rowid, system_id, code, display)
         SELECT id, system_id, code, display FROM concepts WHERE system_id = ?1",
        [system_id],
    ) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(HtsError::StorageError(e.to_string()));
    }

    conn.execute_batch("COMMIT")
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// When `filter_lower` is provided and has ≥ 3 characters, the FTS5 trigram
/// index on `implicit_expansion_fts` is used for fast O(log N) substring
/// matching.  Shorter filters fall back to a LIKE scan (rare in practice).
fn implicit_cache_count(
    conn: &Connection,
    url: &str,
    filter_lower: Option<&str>,
) -> Result<u32, HtsError> {
    let n: i64 = match filter_lower {
        Some(f) if f.len() >= 3 => {
            ensure_implicit_fts(conn, url)?;
            let match_expr = fts5_quote(f);
            conn.query_row(
                "SELECT COUNT(*) FROM implicit_expansion_fts
                 WHERE implicit_expansion_fts MATCH ?1 AND url = ?2",
                rusqlite::params![match_expr, url],
                |r| r.get(0),
            )
        }
        Some(f) => {
            let pattern = format!("%{f}%");
            conn.query_row(
                "SELECT COUNT(*) FROM implicit_expansion_cache
                 WHERE url = ?1
                   AND (LOWER(code) LIKE ?2 OR LOWER(COALESCE(display,'')) LIKE ?2)",
                rusqlite::params![url, pattern],
                |r| r.get(0),
            )
        }
        None => conn.query_row(
            "SELECT COUNT(*) FROM implicit_expansion_cache WHERE url = ?1",
            [url],
            |r| r.get(0),
        ),
    }
    .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(n as u32)
}

/// Return a paginated page of cached entries for an implicit VS URL.
///
/// When `filter_lower` is ≥ 3 characters the FTS5 trigram index is used;
/// shorter filters fall back to a LIKE scan; no filter queries the plain cache.
fn implicit_cache_page(
    conn: &Connection,
    url: &str,
    filter_lower: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ExpansionContains>, HtsError> {
    match filter_lower {
        Some(f) if f.len() >= 3 => {
            ensure_implicit_fts(conn, url)?;
            let match_expr = fts5_quote(f);
            let mut stmt = conn
                .prepare_cached(
                    "SELECT system_url, code, display
                     FROM implicit_expansion_fts
                     WHERE implicit_expansion_fts MATCH ?1 AND url = ?2
                     ORDER BY code
                     LIMIT ?3 OFFSET ?4",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            stmt.query_map(rusqlite::params![match_expr, url, limit, offset], |r| {
                Ok(ExpansionContains {
                    system: r.get(0)?,
                    code: r.get(1)?,
                    display: r.get(2)?,
                    inactive: None,
                    contains: vec![],
                })
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))
        }
        Some(f) => {
            let pattern = format!("%{f}%");
            let mut stmt = conn
                .prepare_cached(
                    "SELECT system_url, code, display
                     FROM implicit_expansion_cache
                     WHERE url = ?1
                       AND (LOWER(code) LIKE ?2 OR LOWER(COALESCE(display,'')) LIKE ?2)
                     ORDER BY system_url, code
                     LIMIT ?3 OFFSET ?4",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            stmt.query_map(rusqlite::params![url, pattern, limit, offset], |r| {
                Ok(ExpansionContains {
                    system: r.get(0)?,
                    code: r.get(1)?,
                    display: r.get(2)?,
                    inactive: None,
                    contains: vec![],
                })
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))
        }
        None => {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT system_url, code, display
                     FROM implicit_expansion_cache
                     WHERE url = ?1
                     ORDER BY system_url, code
                     LIMIT ?2 OFFSET ?3",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            stmt.query_map(rusqlite::params![url, limit, offset], |r| {
                Ok(ExpansionContains {
                    system: r.get(0)?,
                    code: r.get(1)?,
                    display: r.get(2)?,
                    inactive: None,
                    contains: vec![],
                })
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))
        }
    }
}

/// Write computed expansion entries into `implicit_expansion_cache`.
///
/// The DELETE + all INSERTs run inside a single transaction so the cache is
/// always either empty or fully populated — never a partial write.
///
/// The FTS5 mirror (`implicit_expansion_fts`) is **not** populated here; it is
/// built lazily by [`ensure_implicit_fts`] the first time a text-filtered
/// request arrives.  This keeps unfiltered expand requests (e.g. EX01
/// hierarchy expansions) free of FTS5 write overhead.
fn populate_implicit_cache(
    conn: &Connection,
    url: &str,
    codes: &[ExpansionContains],
) -> Result<(), HtsError> {
    // BEGIN IMMEDIATE acquires the write lock upfront so concurrent callers
    // cannot both see an empty cache and then duplicate-write the expansion.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Re-check inside the lock: another VU may have populated this while we
    // were waiting to acquire the write lock.
    let already: bool = match conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM implicit_expansion_cache WHERE url = ?1 LIMIT 1)",
        [url],
        |r| r.get(0),
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
    };

    if already {
        conn.execute_batch("COMMIT")
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        return Ok(());
    }

    if let Err(e) = conn.execute("DELETE FROM implicit_expansion_cache WHERE url = ?1", [url]) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(HtsError::StorageError(e.to_string()));
    }

    {
        let mut stmt = match conn.prepare_cached(
            "INSERT OR IGNORE INTO implicit_expansion_cache
             (url, system_url, code, display)
             VALUES (?1, ?2, ?3, ?4)",
        ) {
            Ok(s) => s,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(HtsError::StorageError(e.to_string()));
            }
        };
        for item in codes {
            if let Err(e) =
                stmt.execute(rusqlite::params![url, item.system, item.code, item.display])
            {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(HtsError::StorageError(e.to_string()));
            }
        }
    }

    conn.execute_batch("COMMIT")
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::sqlite::SqliteTerminologyBackend;
    use crate::import::BundleImportBackend;
    use crate::traits::ValueSetOperations;
    use helios_persistence::tenant::TenantContext;

    fn backend() -> SqliteTerminologyBackend {
        SqliteTerminologyBackend::in_memory().expect("in-memory backend should initialise")
    }

    fn ctx() -> TenantContext {
        TenantContext::system()
    }

    /// Minimal bundle: one CodeSystem (A, B, C) + one ValueSet that explicitly
    /// includes only A and B.
    fn bundle_with_explicit_codes() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs1",
                "url": "http://example.org/cs",
                "version": "1.0",
                "name": "TestCS",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "A", "display": "Concept A" },
                  { "code": "B", "display": "Concept B" },
                  { "code": "C", "display": "Concept C" }
                ]
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs1",
                "url": "http://example.org/vs",
                "name": "TestVS",
                "status": "active",
                "compose": {
                  "include": [
                    {
                      "system": "http://example.org/cs",
                      "concept": [{ "code": "A" }, { "code": "B" }]
                    }
                  ]
                }
              }
            }
          ]
        }"#
    }

    /// Bundle with a ValueSet that includes ALL codes from the CodeSystem.
    fn bundle_with_full_system_include() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs2",
                "url": "http://example.org/cs2",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "X", "display": "Concept X" },
                  { "code": "Y", "display": "Concept Y" }
                ]
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs2",
                "url": "http://example.org/vs2",
                "status": "active",
                "compose": {
                  "include": [{ "system": "http://example.org/cs2" }]
                }
              }
            }
          ]
        }"#
    }

    // ── $expand: explicit code list ────────────────────────────────────────────

    #[tokio::test]
    async fn expand_explicit_codes_returns_correct_concepts() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(2));
        assert_eq!(resp.contains.len(), 2);
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"A"), "A should be in expansion");
        assert!(codes.contains(&"B"), "B should be in expansion");
        assert!(!codes.contains(&"C"), "C should NOT be in expansion");
    }

    // ── $expand: full-system include ───────────────────────────────────────────

    #[tokio::test]
    async fn expand_full_system_include_returns_all_codes() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_full_system_include().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs2".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(2));
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"X"));
        assert!(codes.contains(&"Y"));
    }

    // ── $expand: pagination ────────────────────────────────────────────────────

    #[tokio::test]
    async fn expand_pagination_count_and_offset() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_full_system_include().as_bytes())
            .await
            .unwrap();

        // count=1, offset=0 → first page
        let page1 = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs2".into()),
                    count: Some(1),
                    offset: Some(0),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page1.contains.len(), 1);
        assert_eq!(page1.total, Some(2));

        // count=1, offset=1 → second page
        let page2 = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs2".into()),
                    count: Some(1),
                    offset: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(page2.contains.len(), 1);

        // The two pages should return different codes.
        assert_ne!(
            page1.contains[0].code, page2.contains[0].code,
            "Pages should contain different codes"
        );
    }

    // ── $expand: filter by display substring ──────────────────────────────────

    #[tokio::test]
    async fn expand_filter_by_display_substring() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs".into()),
                    filter: Some("Concept A".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.contains.len(), 1);
        assert_eq!(resp.contains[0].code, "A");
    }

    // ── $expand: cache hit on second call ─────────────────────────────────────

    #[tokio::test]
    async fn expand_cache_hit_on_second_call() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        let req = ExpandRequest {
            url: Some("http://example.org/vs".into()),
            ..Default::default()
        };

        // First call: populates the cache.
        let resp1 = b.expand(&ctx(), req.clone()).await.unwrap();

        // Verify cache was populated.
        {
            let conn = b.pool().get().unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM value_set_expansions WHERE value_set_id = 'vs1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 2, "cache should have 2 entries after first expand");
        }

        // Second call: reads from cache.
        let resp2 = b.expand(&ctx(), req).await.unwrap();
        assert_eq!(resp1.contains.len(), resp2.contains.len());
    }

    // ── $expand: unknown value set ─────────────────────────────────────────────

    #[tokio::test]
    async fn expand_unknown_value_set_returns_not_found() {
        let b = backend();
        let err = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://unknown.org/vs".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HtsError::NotFound(_)));
    }

    // ── $expand: missing url returns InvalidRequest ────────────────────────────

    #[tokio::test]
    async fn expand_missing_url_returns_invalid_request() {
        let b = backend();
        let err = b
            .expand(
                &ctx(),
                ExpandRequest {
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HtsError::InvalidRequest(_)));
    }

    // ── $expand: too-costly limit ─────────────────────────────────────────────

    #[tokio::test]
    async fn expand_exceeds_max_size_returns_too_costly() {
        let b = backend();
        // The bundle_with_full_system_include has 2 codes (X and Y).
        b.import_bundle(&ctx(), bundle_with_full_system_include().as_bytes())
            .await
            .unwrap();

        // Set a limit of 1, which is below the 2-code expansion.
        let err = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs2".into()),
                    max_expansion_size: Some(1),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(
            matches!(err, HtsError::TooCostly(_)),
            "expected TooCostly, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn expand_within_max_size_succeeds() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_full_system_include().as_bytes())
            .await
            .unwrap();

        // Limit of 10 is comfortably above the 2-code expansion.
        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs2".into()),
                    max_expansion_size: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(2));
    }

    // ── $validate-code: code in set ────────────────────────────────────────────

    #[tokio::test]
    async fn validate_code_in_value_set_returns_true() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/vs".into()),
                    code: "A".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(resp.result);
        assert_eq!(resp.display, Some("Concept A".into()));
    }

    // ── $validate-code: code NOT in set ───────────────────────────────────────

    #[tokio::test]
    async fn validate_code_not_in_value_set_returns_false() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/vs".into()),
                    code: "C".into(), // C is in CodeSystem but NOT in the ValueSet
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(!resp.result);
        assert!(resp.message.is_some());
    }

    // ── $validate-code: unknown value set returns 404 ─────────────────────────

    #[tokio::test]
    async fn validate_code_unknown_value_set_returns_not_found() {
        let b = backend();
        let err = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://unknown.org/vs".into()),
                    code: "A".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HtsError::NotFound(_)));
    }

    // ── $validate-code: display mismatch returns false with message ───────────────

    #[tokio::test]
    async fn validate_code_display_mismatch_returns_false_with_message() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/vs".into()),
                    code: "A".into(),
                    display: Some("Wrong Display".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(
            !resp.result,
            "display mismatch makes result=false per FHIR spec"
        );
        assert!(
            resp.message.is_some(),
            "mismatch message should be included"
        );
    }

    // ── $validate-code: display match has no message ───────────────────────────

    #[tokio::test]
    async fn validate_code_display_match_has_no_message() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/vs".into()),
                    code: "A".into(),
                    display: Some("Concept A".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(resp.result);
        assert!(resp.message.is_none(), "no message when display matches");
    }

    // ── $expand: exclude removes codes ────────────────────────────────────────

    #[tokio::test]
    async fn expand_exclude_removes_codes() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-exc",
                "url": "http://example.org/cs-exc",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "P", "display": "P Concept" },
                  { "code": "Q", "display": "Q Concept" },
                  { "code": "R", "display": "R Concept" }
                ]
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs-exc",
                "url": "http://example.org/vs-exc",
                "status": "active",
                "compose": {
                  "include": [{ "system": "http://example.org/cs-exc" }],
                  "exclude": [
                    {
                      "system": "http://example.org/cs-exc",
                      "concept": [{ "code": "Q" }]
                    }
                  ]
                }
              }
            }
          ]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-exc".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"P"));
        assert!(!codes.contains(&"Q"), "Q should be excluded");
        assert!(codes.contains(&"R"));
        assert_eq!(resp.total, Some(2));
    }

    // ── Integration: import Bundle → $expand → $validate-code end-to-end ──────

    #[tokio::test]
    async fn integration_import_expand_validate_code() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        // Expand the value set.
        let expansion = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(expansion.total, Some(2));

        // Validate A (in set) → true.
        let v_in = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/vs".into()),
                    code: "A".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(v_in.result);

        // Validate C (not in set) → false.
        let v_out = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/vs".into()),
                    code: "C".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(!v_out.result);
    }

    // ── implicit ValueSet from CodeSystem.valueSet ────────────────────────────

    /// Bundle with a CodeSystem that declares an implicit ValueSet via `.valueSet`.
    fn bundle_with_implicit_vs() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-impl",
                "url": "http://example.org/cs-impl",
                "valueSet": "http://example.org/vs-impl",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "A", "display": "Concept A" },
                  { "code": "B", "display": "Concept B" },
                  { "code": "C", "display": "Concept C" }
                ]
              }
            }
          ]
        }"#
    }

    #[tokio::test]
    async fn expand_implicit_vs_returns_all_cs_codes() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_implicit_vs().as_bytes())
            .await
            .unwrap();

        // No explicit ValueSet exists — the URL comes from CodeSystem.valueSet
        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-impl".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(3));
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"A"));
        assert!(codes.contains(&"B"));
        assert!(codes.contains(&"C"));
    }

    #[tokio::test]
    async fn expand_implicit_vs_filter_applies() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_implicit_vs().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-impl".into()),
                    filter: Some("Concept A".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.contains.len(), 1);
        assert_eq!(resp.contains[0].code, "A");
    }

    #[tokio::test]
    async fn expand_url_not_matching_any_vs_or_cs_returns_not_found() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_implicit_vs().as_bytes())
            .await
            .unwrap();

        let err = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/no-such".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }

    // ── hierarchical expansion ────────────────────────────────────────────────

    /// Bundle with a CodeSystem that has a 2-level hierarchy (parent → child1, child2).
    fn bundle_with_hierarchy() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-hier",
                "url": "http://example.org/cs-hier",
                "status": "active",
                "content": "complete",
                "concept": [
                  {
                    "code": "root",
                    "display": "Root",
                    "concept": [
                      { "code": "child1", "display": "Child 1" },
                      { "code": "child2", "display": "Child 2" }
                    ]
                  },
                  { "code": "orphan", "display": "Orphan" }
                ]
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs-hier-all",
                "url": "http://example.org/vs-hier-all",
                "status": "active",
                "compose": {
                  "include": [{ "system": "http://example.org/cs-hier" }]
                }
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs-hier-partial",
                "url": "http://example.org/vs-hier-partial",
                "status": "active",
                "compose": {
                  "include": [
                    {
                      "system": "http://example.org/cs-hier",
                      "concept": [{ "code": "child1" }, { "code": "child2" }]
                    }
                  ]
                }
              }
            }
          ]
        }"#
    }

    #[tokio::test]
    async fn expand_hierarchical_true_returns_tree_structure() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_hierarchy().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-hier-all".into()),
                    hierarchical: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Total should equal the flat count (4 codes)
        assert_eq!(resp.total, Some(4));

        // Roots: "orphan" and "root" (both have no parent in the expansion)
        assert_eq!(resp.contains.len(), 2, "expected 2 roots: orphan, root");

        let root = resp
            .contains
            .iter()
            .find(|c| c.code == "root")
            .expect("root should be a root-level entry");

        assert_eq!(root.contains.len(), 2, "root should have 2 children");
        let child_codes: Vec<&str> = root.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(child_codes.contains(&"child1"));
        assert!(child_codes.contains(&"child2"));

        // Orphan should have no children
        let orphan = resp
            .contains
            .iter()
            .find(|c| c.code == "orphan")
            .expect("orphan should be a root-level entry");
        assert!(orphan.contains.is_empty());
    }

    #[tokio::test]
    async fn expand_hierarchical_false_returns_flat_list() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_hierarchy().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-hier-all".into()),
                    hierarchical: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Flat list: all 4 codes, no nesting
        assert_eq!(resp.total, Some(4));
        assert_eq!(resp.contains.len(), 4);
        for c in &resp.contains {
            assert!(c.contains.is_empty(), "flat mode should not nest children");
        }
    }

    #[tokio::test]
    async fn expand_hierarchical_partial_vs_orphans_codes_without_parent() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_hierarchy().as_bytes())
            .await
            .unwrap();

        // vs-hier-partial only includes child1 and child2 (not their parent "root")
        // → both should be roots in the tree
        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-hier-partial".into()),
                    hierarchical: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(2));
        // Both child1 and child2 are roots (parent "root" not in expansion)
        assert_eq!(resp.contains.len(), 2);
        for c in &resp.contains {
            assert!(
                c.contains.is_empty(),
                "children should have no sub-children"
            );
        }
    }

    // ── ?fhir_vs implicit ValueSet URL patterns ───────────────────────────────

    /// Bundle with a simple 3-level hierarchy for testing ?fhir_vs=isa/.
    fn bundle_fhir_vs_hierarchy() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-fvs",
              "url": "http://example.org/cs-fvs",
              "status": "active",
              "content": "complete",
              "concept": [
                {
                  "code": "root",
                  "display": "Root",
                  "concept": [
                    { "code": "child1", "display": "Child 1" },
                    { "code": "child2", "display": "Child 2" }
                  ]
                },
                { "code": "unrelated", "display": "Unrelated" }
              ]
            }
          }]
        }"#
    }

    #[tokio::test]
    async fn expand_fhir_vs_all_concepts() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_fhir_vs_hierarchy().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/cs-fvs?fhir_vs".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(4));
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"root"));
        assert!(codes.contains(&"child1"));
        assert!(codes.contains(&"child2"));
        assert!(codes.contains(&"unrelated"));
    }

    #[tokio::test]
    async fn expand_fhir_vs_isa_returns_descendants() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_fhir_vs_hierarchy().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/cs-fvs?fhir_vs=isa/root".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // << root includes root itself and all descendants (child1, child2)
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"root"), "root should subsume itself");
        assert!(codes.contains(&"child1"));
        assert!(codes.contains(&"child2"));
        assert!(!codes.contains(&"unrelated"), "unrelated is not under root");
    }

    #[tokio::test]
    async fn expand_fhir_vs_unknown_cs_returns_not_found() {
        let b = backend();
        let err = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://no-such.org/cs?fhir_vs".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HtsError::NotFound(_)));
    }

    #[tokio::test]
    async fn validate_code_fhir_vs_all_concepts_code_present() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_fhir_vs_hierarchy().as_bytes())
            .await
            .unwrap();

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/cs-fvs?fhir_vs".into()),
                    code: "child1".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(resp.result);
    }

    #[tokio::test]
    async fn validate_code_fhir_vs_isa_code_in_subtree() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_fhir_vs_hierarchy().as_bytes())
            .await
            .unwrap();

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/cs-fvs?fhir_vs=isa/root".into()),
                    code: "child2".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(resp.result);
    }

    #[tokio::test]
    async fn validate_code_fhir_vs_isa_code_outside_subtree_returns_false() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_fhir_vs_hierarchy().as_bytes())
            .await
            .unwrap();

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/cs-fvs?fhir_vs=isa/root".into()),
                    code: "unrelated".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(!resp.result);
    }

    // ── date parameter (point-in-time filtering for expand) ────────────────────

    /// Seed a code system + value set whose `resource_json` contains a `date`.
    fn seed_dated_vs(b: &SqliteTerminologyBackend, vs_date: &str) {
        let conn = b.pool().get().unwrap();

        let vs_resource_json = serde_json::json!({
            "resourceType": "ValueSet",
            "id": "vs-dated",
            "url": "http://example.org/vs-dated",
            "status": "active",
            "date": vs_date
        })
        .to_string();

        conn.execute_batch(&format!(
            "INSERT INTO code_systems
                 (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('cs-dt', 'http://example.org/cs-dt', NULL, 'DtCS',
                     'active', 'complete', '2024-01-01', '2024-01-01');
             INSERT INTO concepts (id, system_id, code, display)
             VALUES (200, 'cs-dt', 'X', 'X Concept');
             INSERT INTO value_sets
                 (id, url, name, status, compose_json, resource_json, created_at, updated_at)
             VALUES ('vs-dated', 'http://example.org/vs-dated', 'DatedVS', 'active',
                     '{{\"include\":[{{\"system\":\"http://example.org/cs-dt\"}}]}}',
                     '{vs_resource_json}',
                     '2024-01-01', '2024-01-01');",
        ))
        .unwrap();
    }

    #[tokio::test]
    async fn expand_date_after_vs_date_succeeds() {
        let b = backend();
        seed_dated_vs(&b, "2024-06-01");

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-dated".into()),
                    date: Some("2024-12-31".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(1));
        assert_eq!(resp.contains[0].code, "X");
    }

    // ── Inline ValueSet expand (EX02-style) ──────────────────────────────────

    #[tokio::test]
    async fn expand_inline_valueset_with_descendent_of_filter() {
        // Reproduces the EX02 benchmark pattern: POST /ValueSet/$expand with
        // an inline ValueSet resource containing a "descendent-of" filter.
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_hierarchy().as_bytes())
            .await
            .unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-hier",
                    "filter": [{ "property": "concept", "op": "descendent-of", "value": "root" }]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    count: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // descendent-of "root" = strict descendants (child1, child2) but NOT root itself.
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(
            codes.contains(&"child1"),
            "child1 should be a descendant of root"
        );
        assert!(
            codes.contains(&"child2"),
            "child2 should be a descendant of root"
        );
        assert!(
            !codes.contains(&"root"),
            "root itself must not appear (strict descendants)"
        );
        assert!(
            !codes.contains(&"orphan"),
            "orphan is not a descendant of root"
        );
    }

    #[tokio::test]
    async fn expand_inline_valueset_with_generalizes_filter() {
        // generalizes "child1" should return child1 itself plus its ancestors (root).
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_hierarchy().as_bytes())
            .await
            .unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-hier",
                    "filter": [{ "property": "concept", "op": "generalizes", "value": "child1" }]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    count: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(
            codes.contains(&"child1"),
            "child1 itself must be included (self)"
        );
        assert!(
            codes.contains(&"root"),
            "root must be included (ancestor of child1)"
        );
        assert!(
            !codes.contains(&"child2"),
            "child2 is not an ancestor of child1"
        );
        assert!(
            !codes.contains(&"orphan"),
            "orphan is not an ancestor of child1"
        );
    }

    #[tokio::test]
    async fn expand_inline_valueset_unknown_system_total_miss_returns_not_found() {
        // When ALL include clauses reference unknown systems (total miss), the
        // server returns NotFound rather than a silent empty expansion.
        let b = backend();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{ "system": "http://unknown.system/cs" }]
            }
        });

        let err = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    count: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }

    #[tokio::test]
    async fn expand_inline_valueset_partial_miss_returns_results_with_warnings() {
        // When only SOME include clauses reference unknown systems (partial
        // miss), the server returns whatever it can and emits warnings for the
        // skipped systems — matching the FHIR expansion.parameter warning spec.
        let b = backend();

        // Load one of the two referenced systems.
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-known",
              "url": "http://known.system/cs",
              "status": "active", "content": "complete",
              "concept": [{ "code": "K1", "display": "Known One" }]
            }
          }]
        }"#;
        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [
                    { "system": "http://known.system/cs" },
                    { "system": "http://unknown.system/cs" }
                ]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Results from the known system are returned.
        assert_eq!(resp.total, Some(1));
        assert_eq!(resp.contains[0].code, "K1");

        // A warning is emitted for the unknown system.
        assert_eq!(resp.warnings.len(), 1);
        assert!(resp.warnings[0].contains("http://unknown.system/cs"));
    }

    #[tokio::test]
    async fn expand_date_before_vs_date_returns_not_found() {
        let b = backend();
        seed_dated_vs(&b, "2024-06-01");

        // Date before VS date → value set excluded → NotFound → propagates as HtsError.
        let err = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-dated".into()),
                    date: Some("2024-01-01".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }

    // ── EX07: multi-system inline $expand with text filter ────────────────────

    /// Two code systems, three codes each.  An inline ValueSet includes both
    /// systems without an explicit concept list.  A text `filter` should
    /// match only the concepts whose code or display contains the substring,
    /// using SQL pushdown instead of loading all concepts into memory.
    #[tokio::test]
    async fn expand_inline_multisystem_with_text_filter_uses_sql_pushdown() {
        let b = backend();

        let bundle = r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-drugs",
                "url": "http://example.org/drugs",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "AMP01", "display": "Amphetamine base" },
                  { "code": "MET01", "display": "Methylamine compound" },
                  { "code": "COD01", "display": "Codeine" }
                ]
              }
            },
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-obs",
                "url": "http://example.org/observations",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "AMP-OBS", "display": "Amphetamine screening" },
                  { "code": "HRT-OBS", "display": "Heart rate" },
                  { "code": "BP-OBS",  "display": "Blood pressure" }
                ]
              }
            }
          ]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let vs_resource: serde_json::Value = serde_json::from_str(
            r#"{
          "resourceType": "ValueSet",
          "compose": {
            "include": [
              { "system": "http://example.org/drugs" },
              { "system": "http://example.org/observations" }
            ]
          }
        }"#,
        )
        .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(vs_resource),
                    filter: Some("amphetamine".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(
            codes.contains(&"AMP01"),
            "AMP01 display contains 'amphetamine'"
        );
        assert!(
            codes.contains(&"AMP-OBS"),
            "AMP-OBS display contains 'amphetamine'"
        );
        assert!(!codes.contains(&"MET01"), "MET01 should not match");
        assert!(!codes.contains(&"HRT-OBS"), "HRT-OBS should not match");
        assert_eq!(resp.contains.len(), 2);
    }

    /// Filter matching by code (not just display).
    #[tokio::test]
    async fn expand_inline_filter_matches_code_substring() {
        let b = backend();

        let bundle = r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-rx",
              "url": "http://example.org/rx",
              "status": "active",
              "content": "complete",
              "concept": [
                { "code": "AMP01", "display": "Drug one" },
                { "code": "COD01", "display": "Drug two" }
              ]
            }
          }]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let vs_resource: serde_json::Value = serde_json::from_str(
            r#"{
          "resourceType": "ValueSet",
          "compose": { "include": [{ "system": "http://example.org/rx" }] }
        }"#,
        )
        .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(vs_resource),
                    filter: Some("AMP".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.contains.len(), 1);
        assert_eq!(resp.contains[0].code, "AMP01");
    }

    /// Property= filter combined with is-a hierarchy filter: only concepts that
    /// match the property AND are descendants of the root are returned.
    ///
    /// This exercises the property-first filter ordering optimisation — the
    /// property= result is computed first (small, indexed), then ancestry is
    /// checked per candidate (walk UP) rather than expanding all descendants
    /// of the root (walk DOWN).
    #[tokio::test]
    async fn expand_inline_property_and_is_a_filter_intersects_correctly() {
        let b = backend();

        // A code system with:
        //   root → child1 (has prop "kind"="A")
        //         → child2 (has prop "kind"="B")
        //   orphan (has prop "kind"="A", but NOT a descendant of root)
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-prop-hier",
              "url": "http://example.org/cs-prop-hier",
              "status": "active", "content": "complete",
              "property": [{ "code": "kind", "type": "string" }],
              "concept": [
                {
                  "code": "root", "display": "Root",
                  "concept": [
                    { "code": "child1", "display": "Child One",
                      "property": [{ "code": "kind", "valueString": "A" }] },
                    { "code": "child2", "display": "Child Two",
                      "property": [{ "code": "kind", "valueString": "B" }] }
                  ]
                },
                { "code": "orphan", "display": "Orphan",
                  "property": [{ "code": "kind", "valueString": "A" }] }
              ]
            }
          }]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-prop-hier",
                    "filter": [
                        { "property": "kind", "op": "=", "value": "A" },
                        { "property": "concept", "op": "is-a", "value": "root" }
                    ]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    count: Some(20),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        // child1 matches kind=A AND is-a root
        assert!(
            codes.contains(&"child1"),
            "child1 should match (kind=A, descendant of root)"
        );
        // root matches is-a root (self) but has no kind property → excluded
        assert!(
            !codes.contains(&"root"),
            "root has no kind property, should be excluded"
        );
        // child2 has kind=B → excluded by property filter
        assert!(!codes.contains(&"child2"), "child2 has kind=B, not kind=A");
        // orphan has kind=A but is NOT a descendant of root
        assert!(!codes.contains(&"orphan"), "orphan is not under root");
        assert_eq!(
            resp.contains.len(),
            1,
            "only child1 should be in the result"
        );
    }

    /// Inline compose expansion is cached after the first call so that the
    /// second call for the same compose does not recompute the expansion.
    #[tokio::test]
    async fn expand_inline_compose_cached_on_second_call() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_hierarchy().as_bytes())
            .await
            .unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{ "system": "http://example.org/cs-hier" }]
            }
        });

        // First call — populates the cache.
        let resp1 = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Second call — served from cache, result must be identical.
        let resp2 = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp1.total, resp2.total);
        let codes1: Vec<&str> = resp1.contains.iter().map(|c| c.code.as_str()).collect();
        let codes2: Vec<&str> = resp2.contains.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(codes1, codes2);
    }
}
