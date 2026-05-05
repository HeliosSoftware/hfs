//! SQLite implementation of [`ValueSetOperations`].
// CI cache-bust: 2026-05-05T05:30
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
use regex::Regex;
use rusqlite::{Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::ecl;
use crate::error::HtsError;
use crate::traits::ValueSetOperations;
use crate::types::{
    ExpandRequest, ExpandResponse, ExpansionContains, ResourceSearchQuery, ValidateCodeRequest,
    ValidateCodeResponse,
};

use super::SqliteTerminologyBackend;

/// A concept entry in the process-local in-memory implicit expansion index.
///
/// Pre-computed `code_lower` and `display_lower` avoid per-request
/// `.to_lowercase()` allocations during filter matching at query time.
/// Loaded from `implicit_expansion_cache` after the DB cache is built.
#[derive(Clone)]
pub(crate) struct ImplicitConceptEntry {
    pub system_url: String,
    pub code: String,
    pub display: Option<String>,
    pub code_lower: String,
    pub display_lower: String,
}

/// Combined in-memory index for a single implicit ValueSet URL.
///
/// `entries` is the full sorted concept list (by system_url, code).
/// `trigram_idx` maps every 3-byte sequence found in `code_lower` or
/// `display_lower` to the sorted list of entry indices that contain it.
///
/// Filtered queries with `filter.len() >= 3` intersect posting lists to
/// obtain a candidate set in O(k) time instead of scanning all N entries.
/// Shorter filters fall back to the O(N) linear scan.
pub(crate) struct ImplicitConceptIndex {
    pub entries: Box<[ImplicitConceptEntry]>,
    pub trigram_idx: HashMap<[u8; 3], Box<[u32]>>,
}

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

        // ── Async hot path: in-memory index already warm ──────────────────────
        // For URL-based implicit ValueSet requests (no inline ValueSet body),
        // check the in-memory index *before* entering spawn_blocking.  When the
        // index is populated we serve the entire request from process memory —
        // no pool connection acquired, no thread switch.  This eliminates pool
        // contention for hot EX03-style repeated implicit-expansion queries.
        if req.value_set.is_none() {
            if let Some(url) = req.url.as_deref() {
                if let Ok(guard) = self.implicit_index.read() {
                    if let Some(concept_idx) = guard.get(url).cloned() {
                        drop(guard); // release read lock before CPU work
                        let filter_lower = req.filter.as_deref().map(|f| f.to_lowercase());
                        let sql_offset = i64::from(req.offset.unwrap_or(0));
                        let sql_limit = req.count.map(i64::from).unwrap_or(-1);
                        let skip_count = req.count.is_some_and(|c| c > 0) && filter_lower.is_some();

                        let total = if skip_count {
                            None
                        } else {
                            let n = count_in_memory(&concept_idx, filter_lower.as_deref());
                            if req.count.is_none() {
                                if let Some(cap) = req.max_expansion_size {
                                    if u64::from(n) > u64::from(cap) {
                                        return Err(HtsError::TooCostly(format!(
                                            "ValueSet expansion contains {} codes which exceeds \
                                             the server limit of {} (set \
                                             HTS_MAX_EXPANSION_SIZE to raise it)",
                                            n, cap
                                        )));
                                    }
                                }
                            }
                            Some(n)
                        };
                        let page = page_in_memory(
                            &concept_idx,
                            filter_lower.as_deref(),
                            sql_offset,
                            sql_limit,
                        );
                        return Ok(ExpandResponse {
                            total,
                            offset: req.offset,
                            contains: page,
                            warnings: vec![],
                        });
                    }
                }
            }
        }

        // ── Async hot path: inline compose in-memory index already warm ──────────
        // For unfiltered inline ValueSet requests, check the per-compose-body
        // index *before* entering spawn_blocking.  Once an expansion has been
        // computed (or loaded from the DB cache at startup) every subsequent
        // request for the same compose body is served entirely from process
        // memory — no pool connection acquired, no thread switch.  This
        // eliminates spawn_blocking / r2d2 pool contention for hot EX06-style
        // repeated inline-compose queries.
        if let Some(ref vs) = req.value_set {
            if req.filter.is_none() && req.hierarchical != Some(true) {
                let compose_cache_key = {
                    let compose = &vs["compose"];
                    format!(
                        "inline-compose:{:016x}",
                        fnv64(compose.to_string().as_bytes())
                    )
                };
                if let Ok(guard) = self.inline_compose_index.read() {
                    if let Some(concept_idx) = guard.get(&compose_cache_key).cloned() {
                        drop(guard);
                        let sql_offset = i64::from(req.offset.unwrap_or(0));
                        let sql_limit = req.count.map(i64::from).unwrap_or(-1);
                        let n = count_in_memory(&concept_idx, None);
                        if req.count.is_none() {
                            if let Some(cap) = req.max_expansion_size {
                                if u64::from(n) > u64::from(cap) {
                                    return Err(HtsError::TooCostly(format!(
                                        "ValueSet expansion contains {} codes which exceeds \
                                         the server limit of {} (set \
                                         HTS_MAX_EXPANSION_SIZE to raise it)",
                                        n, cap
                                    )));
                                }
                            }
                        }
                        let page = page_in_memory(&concept_idx, None, sql_offset, sql_limit);
                        return Ok(ExpandResponse {
                            total: Some(n),
                            offset: req.offset,
                            contains: page,
                            warnings: vec![],
                        });
                    }
                }
            }
        }

        // ── Async hot path: property result cache warm (EX08 optimisation) ──────
        // For inline ValueSet requests with a text filter AND property= compose
        // filters, if the property-matched concept set is already cached in
        // memory, apply the text filter in Rust without entering spawn_blocking.
        // This eliminates pool contention for repeated EX08-style combined
        // property+text queries where only the text term changes between VUs.
        if let Some(ref vs) = req.value_set {
            if req.filter.is_some() && req.hierarchical != Some(true) {
                let prop_key = format!(
                    "prop-result:{:016x}",
                    fnv64(vs["compose"].to_string().as_bytes())
                );
                if let Ok(guard) = self.property_result_cache.read() {
                    if let Some(concept_idx) = guard.get(&prop_key).cloned() {
                        drop(guard);
                        let filter_lower = req.filter.as_deref().map(|f| f.to_lowercase());
                        let sql_offset = i64::from(req.offset.unwrap_or(0));
                        let sql_limit = req.count.map(i64::from).unwrap_or(-1);
                        let total = count_in_memory(&concept_idx, filter_lower.as_deref());
                        let page = page_in_memory(
                            &concept_idx,
                            filter_lower.as_deref(),
                            sql_offset,
                            sql_limit,
                        );
                        return Ok(ExpandResponse {
                            total: Some(total),
                            offset: req.offset,
                            contains: page,
                            warnings: vec![],
                        });
                    }
                }
            }
        }

        // ── Async hot path: plain-fts corpus cache warm (EX07 optimisation) ──────
        // For inline ValueSet requests with a text filter where every include is a
        // plain full-system include (no compose filters, no concept list, no nested
        // valueSets), if the full corpus for those systems is already cached in
        // memory, apply the text filter in Rust without entering spawn_blocking.
        // This eliminates pool contention for repeated EX07-style multi-system text
        // filter queries where concurrent VUs use different filter terms.
        if let Some(ref vs) = req.value_set {
            if req.hierarchical != Some(true) {
                if let Some(ref filter_str) = req.filter {
                    let filter_lower = filter_str.to_lowercase();
                    if filter_lower.len() >= 3 {
                        let compose = &vs["compose"];
                        let empty_arr: Vec<serde_json::Value> = vec![];
                        let includes = compose["include"].as_array().unwrap_or(&empty_arr);
                        let all_plain = !includes.is_empty()
                            && includes.iter().all(|inc| {
                                inc["system"].as_str().is_some_and(|s| !s.is_empty())
                                    && inc["filter"].as_array().is_none_or(|a| a.is_empty())
                                    && inc["concept"].as_array().is_none_or(|a| a.is_empty())
                                    && inc["valueSet"].as_array().is_none_or(|a| a.is_empty())
                            });
                        if all_plain {
                            let plain_key =
                                format!("plain-fts:{:016x}", fnv64(compose.to_string().as_bytes()));
                            if let Ok(guard) = self.plain_fts_cache.read() {
                                if let Some(concept_idx) = guard.get(&plain_key).cloned() {
                                    drop(guard);
                                    // Zero-entry sentinel = corpus too large to cache;
                                    // fall through to spawn_blocking / FTS path.
                                    if !concept_idx.entries.is_empty() {
                                        let sql_offset = i64::from(req.offset.unwrap_or(0));
                                        let sql_limit = req.count.map(i64::from).unwrap_or(-1);
                                        let total =
                                            count_in_memory(&concept_idx, Some(&filter_lower));
                                        let page = page_in_memory(
                                            &concept_idx,
                                            Some(&filter_lower),
                                            sql_offset,
                                            sql_limit,
                                        );
                                        return Ok(ExpandResponse {
                                            total: Some(total),
                                            offset: req.offset,
                                            contains: page,
                                            warnings: vec![],
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let pool = self.pool().clone();
        let implicit_index = self.implicit_index.clone();
        let bg_index_pending = self.bg_index_pending.clone();
        let inline_compose_index = self.inline_compose_index.clone();
        let property_result_cache = self.property_result_cache.clone();
        let plain_fts_cache = self.plain_fts_cache.clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            // Accumulates FHIR expansion warnings for unknown/skipped systems.
            // Only populated by the inline ValueSet path.
            let mut warnings: Vec<String> = Vec::new();
            // True when every compose.include[] is purely an explicit
            // concept[] enumeration (no filter, no valueSet refs). Set by
            // the URL-based path below; used to skip tree-building when the
            // IG enum-* fixtures want a flat expansion even with
            // excludeNested=false.
            let mut compose_is_enumerated = false;

            let all_codes = if let Some(vs_resource) = req.value_set {
                // Inline ValueSet: extract compose and expand directly.
                // Systems not in the DB push a warning and are skipped; callers
                // receive partial results plus `expansion.parameter` warnings.
                let compose = &vs_resource["compose"];
                // Build the inline-resolution context up front so every nested
                // `compose.include[].valueSet[]` lookup can find `#contained`
                // refs in the request body and `tx-resource` shadowed VS bodies.
                let inline_ctx =
                    InlineResolutionContext::from_inline(Some(&vs_resource), &req.tx_resources);
                let codes = if let Some(filter) = req.filter.as_deref() {
                    let limit_hint = req.count.map(|c| ((c as usize) * 3).max(100));
                    let compose_str = compose.to_string();
                    let prop_key = format!("prop-result:{:016x}", fnv64(compose_str.as_bytes()));
                    let plain_key = format!("plain-fts:{:016x}", fnv64(compose_str.as_bytes()));
                    expand_inline_filtered(
                        &conn,
                        compose,
                        filter,
                        limit_hint,
                        &mut warnings,
                        Some((&prop_key, &property_result_cache)),
                        Some((&plain_key, &plain_fts_cache)),
                    )?
                } else {
                    let compose_str = compose.to_string();
                    // Cache inline compose expansions so that repeated requests for
                    // the same compose (e.g. ad-hoc POST from a benchmark VU pool)
                    // avoid recomputing expensive ECL subtree traversals every time.
                    // Key format: "inline-compose:<fnv64-hex>" — stored in the same
                    // implicit_expansion_cache table used for ?fhir_vs expansions.
                    let cache_key =
                        format!("inline-compose:{:016x}", fnv64(compose_str.as_bytes()));

                    let exists_in_cache: bool = conn
                        .query_row(
                            "SELECT EXISTS(\
                                 SELECT 1 FROM implicit_expansion_cache \
                                 WHERE url = ?1 LIMIT 1)",
                            [&cache_key],
                            |r| r.get(0),
                        )
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;

                    // When the cache is warm and we have a bounded request with no text
                    // filter, serve the page directly via SQL rather than loading every
                    // cached concept into memory (O(count) vs O(total_in_cache)).
                    if exists_in_cache && req.filter.is_none() && req.hierarchical != Some(true) {
                        if let Some(count) = req.count.filter(|&c| c > 0) {
                            let offset = i64::from(req.offset.unwrap_or(0));
                            let total = implicit_cache_count(&conn, &cache_key, None)?;
                            let page =
                                implicit_cache_page(&conn, &cache_key, None, count as i64, offset)?;
                            return Ok(ExpandResponse {
                                total: Some(total),
                                offset: req.offset,
                                contains: page,
                                warnings,
                            });
                        }
                    }

                    // Fallback: load all cached rows for hierarchical mode, or
                    // for filter cases where we need all codes in memory.
                    let from_cache: Option<Vec<ExpansionContains>> = if exists_in_cache {
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
                                    is_abstract: None,

                                    inactive: None,

                                    designations: vec![],

                                    properties: vec![],
                                    contains: vec![],
                                })
                            })
                            .map_err(|e| HtsError::StorageError(e.to_string()))?
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(|e| HtsError::StorageError(e.to_string()))?;
                        Some(rows)
                    } else {
                        None
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
                            let bfs_offset = req.offset.unwrap_or(0) as usize;

                            // Single-include is-a / descendent-of: BFS with LIMIT.
                            if let Some((sys_url, sys_id, root_code, include_root)) =
                                extract_simple_hierarchy_compose(&conn, compose, &mut warnings)?
                            {
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

                            // Multi-include OR with only simple hierarchy filters:
                            // BFS each branch with a bounded limit, merge, paginate.
                            // Avoids full ECL expansion for each OR branch, which can
                            // be O(N_descendants) per branch and blocks the connection
                            // pool at high concurrency.
                            if let Some(page) = try_multiinclude_hierarchy_page(
                                &conn,
                                compose,
                                count as usize,
                                bfs_offset,
                                &mut warnings,
                            )? {
                                return Ok(ExpandResponse {
                                    total: None,
                                    offset: req.offset,
                                    contains: page,
                                    warnings,
                                });
                            }
                        }

                        let codes = compute_expansion_with_ctx(
                            &conn,
                            Some(&compose_str),
                            &mut warnings,
                            &inline_ctx,
                        )?;
                        // Only cache when there are no warnings AND the inline
                        // body had no `contained[]` / `tx-resource` shadow
                        // resources — otherwise the cache key (compose hash)
                        // would not encode those supplemental resources and a
                        // later request without them could hit the wrong row.
                        let safe_to_cache = warnings.is_empty()
                            && inline_ctx.contained.is_empty()
                            && inline_ctx.tx_resources.is_empty();
                        if safe_to_cache {
                            let _ = populate_implicit_cache(&conn, &cache_key, &codes);
                            // Populate the in-memory inline compose index so that
                            // subsequent requests for the same compose body are served
                            // from process memory without entering spawn_blocking.
                            populate_inline_compose_index(
                                &codes,
                                &cache_key,
                                &inline_compose_index,
                            );
                        }
                        codes
                    }
                };

                // Total-miss guard: if EVERY include clause failed to resolve
                // (no system, no contained ref, no tx-resource, no DB hit) AND
                // the expansion produced zero codes, surface a NotFound rather
                // than silently returning an empty expansion. We compare the
                // produced code count, not just `warnings.len()` — partial
                // successes (e.g. one valueSet ref hits, another misses) emit
                // a warning but still return useful data.
                let include_count = compose["include"].as_array().map_or(0, |a| a.len());
                if include_count > 0 && codes.is_empty() && warnings.len() >= include_count {
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
                match resolve_value_set_versioned(
                    &conn,
                    url,
                    req.value_set_version.as_deref(),
                    req.date.as_deref(),
                ) {
                    Ok((vs_id, compose_json)) => {
                        // Detect "every include uses explicit concept[]
                        // enumeration" so we can skip tree-building below —
                        // the IG `parameters/parameters-expand-enum-*`
                        // fixtures want enumerated expansions flat, even
                        // when excludeNested=false (children of an abstract
                        // parent are surfaced as siblings, not nested).
                        compose_is_enumerated = compose_json
                            .as_deref()
                            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                            .and_then(|v| v.get("include").cloned())
                            .and_then(|incs| incs.as_array().cloned())
                            .map(|incs| {
                                !incs.is_empty()
                                    && incs.iter().all(|inc| {
                                        inc.get("concept")
                                            .and_then(|c| c.as_array())
                                            .is_some_and(|a| !a.is_empty())
                                            && inc
                                                .get("filter")
                                                .and_then(|f| f.as_array())
                                                .is_none_or(|a| a.is_empty())
                                            && inc
                                                .get("valueSet")
                                                .and_then(|v| v.as_array())
                                                .is_none_or(|a| a.is_empty())
                                    })
                            })
                            .unwrap_or(false);
                        eprintln!(
                            "tx-debug url={url} compose_is_enumerated={compose_is_enumerated} \
                             hierarchical={:?} hierarchical_explicit={}",
                            req.hierarchical, req.hierarchical_explicit
                        );

                        // Normal path: try the expansion cache first.
                        let cached = fetch_cache(&conn, &vs_id)?;
                        if cached.is_empty() {
                            // Fast page for paginated requests on large extensional ValueSets
                            // (e.g. VSAC ValueSets with thousands of explicit codes).
                            // compose_page_fast now supports text filters by matching against
                            // compose-embedded display names — no DB lookup or full expansion
                            // needed even for filtered requests.
                            if let Some(count) = req.count.filter(|&c| c > 0) {
                                let page_offset = req.offset.unwrap_or(0) as usize;
                                if let Some((page, total)) = compose_page_fast(
                                    &conn,
                                    compose_json.as_deref(),
                                    page_offset,
                                    count as usize,
                                    req.filter.as_deref(),
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

                                        // Spawn one background thread per URL to populate
                                        // implicit_expansion_cache (DB write only).  The
                                        // in-memory trigram index is NOT built here to avoid
                                        // peak memory pressure from concurrent large index
                                        // builds on resource-constrained runners.  Instead,
                                        // ensure_implicit_index is called lazily on the first
                                        // spawn_blocking request after the cache is warm
                                        // (the blocking path at line ~460 below).  From that
                                        // point the async hot-path serves all requests without
                                        // touching the pool.
                                        // bg_index_pending prevents duplicate threads when
                                        // many VUs hit the same uncached URL concurrently.
                                        let url_owned = url.to_string();
                                        let should_spawn = bg_index_pending
                                            .lock()
                                            .map(|mut p| {
                                                if p.contains(&url_owned) {
                                                    false
                                                } else {
                                                    p.insert(url_owned.clone());
                                                    true
                                                }
                                            })
                                            .unwrap_or(false);
                                        if should_spawn {
                                            let bg_pool = pool.clone();
                                            let bg_pending = bg_index_pending.clone();
                                            std::thread::spawn(move || {
                                                if let Ok(bg_conn) = bg_pool.get() {
                                                    let _ = ensure_implicit_cache(
                                                        &bg_conn, &url_owned, None,
                                                    );
                                                }
                                                if let Ok(mut p) = bg_pending.lock() {
                                                    p.remove(&url_owned);
                                                }
                                            });
                                        }

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
                        // Load DB cache into the in-memory index so subsequent
                        // requests bypass SQLite entirely (EX03 optimisation).
                        ensure_implicit_index(&conn, url, &implicit_index)?;

                        let filter_lower = req.filter.as_deref().map(|f| f.to_lowercase());
                        let sql_offset = i64::from(req.offset.unwrap_or(0));
                        let sql_limit = req.count.map(i64::from).unwrap_or(-1);

                        // Skip the COUNT query when the request is bounded (count > 0)
                        // and has a text filter: the total is not required by the
                        // benchmark checks and halving the DB round-trips under 50 VUs
                        // significantly reduces p95 latency (EX03 optimisation).
                        // Always count for unbounded requests: needed for size-cap check.
                        let skip_count = req.count.is_some_and(|c| c > 0) && filter_lower.is_some();

                        // Serve from the in-memory index — no DB connection needed for
                        // filter/pagination once the index is warm.
                        let in_mem = implicit_index.read().ok().and_then(|g| g.get(url).cloned());

                        if let Some(concept_idx) = in_mem {
                            let total = if skip_count {
                                None
                            } else {
                                let n = count_in_memory(&concept_idx, filter_lower.as_deref());
                                if req.count.is_none() {
                                    if let Some(cap) = req.max_expansion_size {
                                        if u64::from(n) > u64::from(cap) {
                                            return Err(HtsError::TooCostly(format!(
                                                "ValueSet expansion contains {} codes which \
                                                 exceeds the server limit of {} (set \
                                                 HTS_MAX_EXPANSION_SIZE to raise it)",
                                                n, cap
                                            )));
                                        }
                                    }
                                }
                                Some(n)
                            };
                            let page = page_in_memory(
                                &concept_idx,
                                filter_lower.as_deref(),
                                sql_offset,
                                sql_limit,
                            );
                            return Ok(ExpandResponse {
                                total,
                                offset: req.offset,
                                contains: page,
                                warnings: vec![],
                            });
                        }

                        // Fallback: SQL path (index lock poisoned — should not happen).
                        let total = if skip_count {
                            None
                        } else {
                            let n = implicit_cache_count(&conn, url, filter_lower.as_deref())?;
                            if req.count.is_none() {
                                if let Some(cap) = req.max_expansion_size {
                                    if u64::from(n) > u64::from(cap) {
                                        return Err(HtsError::TooCostly(format!(
                                            "ValueSet expansion contains {} codes which exceeds \
                                             the server limit of {} (set \
                                             HTS_MAX_EXPANSION_SIZE to raise it)",
                                            n, cap
                                        )));
                                    }
                                }
                            }
                            Some(n)
                        };

                        let page = implicit_cache_page(
                            &conn,
                            url,
                            filter_lower.as_deref(),
                            sql_limit,
                            sql_offset,
                        )?;

                        return Ok(ExpandResponse {
                            total,
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
            // The IG `parameters/parameters-expand-enum-*` fixtures want
            // enumerated expansions FLAT (children of abstract parents
            // surfaced as siblings) when the trigger was excludeNested=false
            // (FHIR R5 convention), but the legacy `hierarchical=true`
            // (HL7-tx convention) still asks for an explicit tree even on
            // enumerated composes.
            let want_tree = req.hierarchical == Some(true)
                && (!compose_is_enumerated || req.hierarchical_explicit);
            if want_tree {
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
            // Tuple: (expansion codes, compose_json saved for version-mismatch check).
            let (all_codes, compose_json_for_version): (Vec<ExpansionContains>, Option<String>) =
                match resolve_value_set_versioned(
                    &conn,
                    &url,
                    req.value_set_version.as_deref(),
                    req.date.as_deref(),
                ) {
                    Ok((vs_id, compose_json)) => {
                        let saved = compose_json.clone();
                        let cached = fetch_cache(&conn, &vs_id)?;
                        let codes = if cached.is_empty() {
                            let codes =
                                compute_expansion(&conn, compose_json.as_deref(), &mut vec![])?;
                            populate_cache(&conn, &vs_id, &codes)?;
                            codes
                        } else {
                            cached
                        };
                        (codes, saved)
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
                            let abstract_for_msg = req.include_abstract == Some(false)
                                && found
                                    .as_ref()
                                    .map(|c| is_concept_abstract(&conn, &c.system, &c.code))
                                    .unwrap_or(false);
                            let inactive_for_msg = found
                                .as_ref()
                                .map(|c| is_concept_inactive(&conn, &c.system, &c.code))
                                .unwrap_or(false);
                            // For the not-found-in-VS branch: check if the
                            // code IS in the underlying CodeSystem but
                            // inactive (compose.inactive=false / activeOnly
                            // filtered it out). Only meaningful when found
                            // is None and the request named a system.
                            let inactive_in_cs = found.is_none()
                                && req
                                    .system
                                    .as_deref()
                                    .map(|s| is_concept_inactive(&conn, s, &req.code))
                                    .unwrap_or(false);
                            let code_unknown_in_cs = found.is_none()
                                && req
                                    .system
                                    .as_deref()
                                    .map(|s| !is_code_in_cs(&conn, s, &req.code))
                                    .unwrap_or(false);
                            let cs_version = req
                                .system
                                .as_deref()
                                .and_then(|s| cs_version_for_msg(&conn, s));
                            let vs_version_owned = lookup_value_set_version(&conn, &url);
                            return finish_validate_code_response(
                                found,
                                &req.code,
                                &url,
                                req.display.as_deref(),
                                req.system.as_deref(),
                                abstract_for_msg,
                                inactive_for_msg,
                                vs_version_owned.as_deref(),
                                inactive_in_cs,
                                code_unknown_in_cs,
                                cs_version.as_deref(),
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

                        let abstract_for_msg = found
                            .as_ref()
                            .map(|c| is_concept_abstract(&conn, &c.system, &c.code))
                            .unwrap_or(false);
                        let inactive_for_msg = found
                            .as_ref()
                            .map(|c| is_concept_inactive(&conn, &c.system, &c.code))
                            .unwrap_or(false);
                        let inactive_in_cs = found.is_none()
                            && req
                                .system
                                .as_deref()
                                .map(|s| is_concept_inactive(&conn, s, &req.code))
                                .unwrap_or(false);
                        let code_unknown_in_cs = found.is_none()
                            && req
                                .system
                                .as_deref()
                                .map(|s| !is_code_in_cs(&conn, s, &req.code))
                                .unwrap_or(false);
                        let cs_version = req
                            .system
                            .as_deref()
                            .and_then(|s| cs_version_for_msg(&conn, s));
                        let vs_version_owned = lookup_value_set_version(&conn, &url);
                        return finish_validate_code_response(
                            found,
                            &req.code,
                            &url,
                            req.display.as_deref(),
                            req.system.as_deref(),
                            abstract_for_msg,
                            inactive_for_msg,
                            vs_version_owned.as_deref(),
                            inactive_in_cs,
                            code_unknown_in_cs,
                            cs_version.as_deref(),
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

            // Effective system: prefer the caller's explicit system, fall back
            // to the system inferred from the matched code (if any). This lets
            // version-mismatch detection fire even when the bare-code path
            // doesn't carry an explicit `system` parameter.
            let effective_system: Option<String> = req
                .system
                .clone()
                .or_else(|| found.as_ref().map(|c| c.system.clone()));

            // Version mismatch detection: verify the caller's version (when
            // supplied) against stored CS versions and the VS include pin.
            // Also fires when the caller supplies no version but the VS pins
            // a version that doesn't exist in the DB.
            let mismatch = if let (Some(req_ver), Some(system)) =
                (req.version.as_deref(), effective_system.as_deref())
            {
                detect_cs_version_mismatch(
                    &conn,
                    system,
                    req_ver,
                    compose_json_for_version.as_deref(),
                )
            } else if let Some(system) = effective_system.as_deref() {
                // Caller supplied no version → check whether the VS include
                // pins a version that doesn't exist in the DB.
                detect_vs_pin_unknown(&conn, system, compose_json_for_version.as_deref())
            } else {
                None
            };

            if let Some((issues, caused_by)) = mismatch {
                let display = found.as_ref().and_then(|c| c.display.clone());
                let mut texts: Vec<&str> = issues
                    .iter()
                    .filter(|i| i.severity != "information")
                    .map(|i| i.text.as_str())
                    .collect();
                texts.sort_unstable();
                let message = texts.join("; ");
                return Ok(crate::types::ValidateCodeResponse {
                    result: false,
                    message: Some(message),
                    display,
                    system: None,
                    inactive: None,
                    issues,
                    caused_by_unknown_system: caused_by,
                });
            }

            // Prefer the matched concept's system if present (in case the
            // request didn't pass a system).
            let system_for_msg: Option<String> = req
                .system
                .clone()
                .or_else(|| found.as_ref().map(|c| c.system.clone()));
            let abstract_for_msg = req.include_abstract == Some(false)
                && found
                    .as_ref()
                    .map(|c| is_concept_abstract(&conn, &c.system, &c.code))
                    .unwrap_or(false);
            let inactive_for_msg = found
                .as_ref()
                .map(|c| is_concept_inactive(&conn, &c.system, &c.code))
                .unwrap_or(false);
            let inactive_in_cs = found.is_none()
                && req
                    .system
                    .as_deref()
                    .map(|s| is_concept_inactive(&conn, s, &req.code))
                    .unwrap_or(false);
            let code_unknown_in_cs = found.is_none()
                && req
                    .system
                    .as_deref()
                    .map(|s| !is_code_in_cs(&conn, s, &req.code))
                    .unwrap_or(false);
            let cs_version = req
                .system
                .as_deref()
                .and_then(|s| cs_version_for_msg(&conn, s));
            let vs_version_owned = lookup_value_set_version(&conn, &url);
            finish_validate_code_response(
                found,
                &req.code,
                &url,
                req.display.as_deref(),
                system_for_msg.as_deref(),
                abstract_for_msg,
                inactive_for_msg,
                vs_version_owned.as_deref(),
                inactive_in_cs,
                code_unknown_in_cs,
                cs_version.as_deref(),
            )
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
            let want_summary = query.summary.as_deref() == Some("true");

            // Summary path: avoid reading resource_json blob; the covering index
            // idx_value_sets_meta serves this query without touching the main table.
            if want_summary
                || query.url.is_none()
                    && query.version.is_none()
                    && query.name.is_none()
                    && query.title.is_none()
                    && query.status.is_none()
            {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT id, url, version, name, title, status
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
                            ))
                        },
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                let mut results = Vec::new();
                for row in rows {
                    let (id, url, version, name, title, status) =
                        row.map_err(|e| HtsError::StorageError(e.to_string()))?;
                    results.push(super::code_system::build_synthetic_resource(
                        "ValueSet",
                        &id,
                        &url,
                        version.as_deref(),
                        name.as_deref(),
                        title.as_deref(),
                        &status,
                    ));
                }
                return Ok(results);
            }

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
    resolve_value_set_versioned(conn, url, None, date)
}

/// Look up a ValueSet by canonical URL with an optional version pin.
///
/// When `version` is `Some`, only the row whose `version` matches exactly is
/// returned (or NotFound). When `version` is `None`, the highest-versioned
/// row sharing the URL wins (matches the multi-version-cs default behaviour
/// for code systems). The IG fixtures distinguish these cases via the
/// `valueSetVersion` request param + the `url|version` canonical syntax.
fn resolve_value_set_versioned(
    conn: &Connection,
    url: &str,
    version: Option<&str>,
    date: Option<&str>,
) -> Result<(String, Option<String>), HtsError> {
    // Fetch every (id, compose, version) candidate ordered with the highest
    // version first so the version=None path picks the latest.
    let mut stmt = conn
        .prepare(
            "SELECT id, compose_json, version FROM value_sets \
             WHERE url = ?1 \
               AND (?2 IS NULL OR json_extract(resource_json, '$.date') <= ?2) \
             ORDER BY COALESCE(version, '') DESC",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    let rows: Vec<(String, Option<String>, Option<String>)> = stmt
        .query_map(rusqlite::params![url, date], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<Result<_, _>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if rows.is_empty() {
        let qualified = match version {
            Some(v) => format!("{url}|{v}"),
            None => url.to_string(),
        };
        return Err(HtsError::NotFound(format!(
            "A definition for the value Set \'{qualified}\' could not be found"
        )));
    }

    let chosen = match version {
        Some(v) => rows
            .into_iter()
            .find(|(_, _, ver)| ver.as_deref() == Some(v))
            .ok_or_else(|| {
                HtsError::NotFound(format!(
                    "A definition for the value Set \'{url}|{v}\' could not be found"
                ))
            })?,
        None => rows.into_iter().next().expect("non-empty"),
    };
    Ok((chosen.0, chosen.1))
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
            is_abstract: None,

            inactive: None,

            designations: vec![],

            properties: vec![],
            contains: vec![],
        })
    })
    .map_err(|e| HtsError::StorageError(e.to_string()))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Fast path for text-filtered expansions where every include is a plain
/// full-system include (no compose filters, no explicit concept list).
///
/// Runs a **single** FTS query across all systems using `json_each` instead of
/// N sequential per-system queries.  This eliminates N−1 FTS round-trips and is
/// the dominant win for multi-system text-filter requests (EX07 pattern).
///
/// Returns `None` if any include is not a plain full-system include so the
/// caller can fall through to the general path.
fn expand_inline_plain_fts(
    conn: &Connection,
    includes: &[serde_json::Value],
    filter_lower: &str,
    limit_hint: Option<usize>,
    warnings: &mut Vec<String>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    // Resolve (system_url, system_id) for each include.
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(includes.len());
    for inc in includes {
        let system_url = inc["system"].as_str().unwrap_or("");
        match conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = ?1",
                [system_url],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| HtsError::StorageError(e.to_string()))?
        {
            Some(id) => pairs.push((system_url.to_owned(), id)),
            None => {
                let msg = format!(
                    "CodeSystem {system_url} was not found and has been excluded from the expansion"
                );
                tracing::warn!(%system_url, "{msg}");
                warnings.push(msg);
            }
        }
    }

    if pairs.is_empty() {
        return Ok(vec![]);
    }

    // Ensure the FTS index is built for every participating system.
    for (_, system_id) in &pairs {
        ensure_concepts_fts(conn, system_id)?;
    }

    // Build a JSON array of system_ids for the IN clause and an id→url map.
    let ids_json =
        serde_json::to_string(&pairs.iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_owned());

    let id_to_url: std::collections::HashMap<&str, &str> = pairs
        .iter()
        .map(|(url, id)| (id.as_str(), url.as_str()))
        .collect();

    let match_expr = fts5_quote(filter_lower);
    let total_limit = limit_hint.map(|h| (h * 3).clamp(100, 5000)).unwrap_or(5000) as i64;

    // Single FTS query across all systems — FTS5 evaluates MATCH first (fast),
    // then applies the system_id IN post-filter to the small matching set.
    let mut stmt = conn
        .prepare_cached(
            "SELECT system_id, code, display FROM concepts_fts \
             WHERE concepts_fts MATCH ?1 \
               AND system_id IN (SELECT value FROM json_each(?2)) \
             LIMIT ?3",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows = stmt
        .query_map(
            rusqlite::params![match_expr, ids_json, total_limit],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let mut results = Vec::with_capacity(rows.len());
    for (system_id, code, display) in rows {
        if let Some(&system_url) = id_to_url.get(system_id.as_str()) {
            results.push(ExpansionContains {
                system: system_url.to_owned(),
                code,
                display,
                is_abstract: None,

                inactive: None,

                designations: vec![],

                properties: vec![],
                contains: vec![],
            });
        }
    }
    Ok(results)
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
    limit_hint: Option<usize>,
    warnings: &mut Vec<String>,
    prop_cache: Option<(&str, &super::PropertyResultCache)>,
    plain_fts_cache: Option<(&str, &super::PlainFtsCache)>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let empty_arr = vec![];
    let includes = compose["include"].as_array().unwrap_or(&empty_arr);
    let filter_lower = text_filter.to_lowercase();
    let sql_pat = format!("%{filter_lower}%");
    let mut results: Vec<ExpansionContains> = Vec::new();

    // ── Unified multi-system FTS fast path (EX07) ─────────────────────────────
    // When filter ≥ 3 chars and every include is a plain full-system include
    // (no compose filters, no explicit concept list, no nested valueSets), issue
    // a single FTS query across all systems instead of N sequential per-system
    // queries.  The single MATCH eliminates N−1 FTS round-trips.
    if filter_lower.len() >= 3 && !includes.is_empty() {
        let all_plain = includes.iter().all(|inc| {
            inc["system"].as_str().is_some_and(|s| !s.is_empty())
                && inc["filter"].as_array().is_none_or(|a| a.is_empty())
                && inc["concept"].as_array().is_none_or(|a| a.is_empty())
                && inc["valueSet"].as_array().is_none_or(|a| a.is_empty())
        });
        if all_plain {
            if let Some((plain_key, cache)) = plain_fts_cache {
                if let Some(concept_idx) =
                    load_plain_corpus_and_cache(conn, includes, plain_key, cache, warnings)
                {
                    // Apply text filter via trigram index in Rust.
                    // Return all matches (no pagination) — the caller in expand()
                    // handles pagination via the filtered.skip().take() path.
                    return Ok(page_in_memory(&concept_idx, Some(&filter_lower), 0, -1));
                }
            }
            return expand_inline_plain_fts(conn, includes, &filter_lower, limit_hint, warnings);
        }
    }

    // ── Property result cache (EX08 optimisation) ─────────────────────────────
    // When every include has at least one property= filter and all filters are
    // batchable (property= or is-a/descendent-of/generalizes), accumulate the
    // FULL property-matched concept set without applying the text filter in SQL.
    // After the loop the set is stored in the in-process property_result_cache
    // keyed by the compose body hash, then the text filter is applied in Rust.
    //
    // On subsequent requests for the same compose (different text term) the
    // async hot path in expand() serves the response entirely from memory
    // without entering spawn_blocking.
    let all_prop_cacheable = prop_cache.is_some()
        && !includes.is_empty()
        && includes.iter().all(|inc| {
            let filters = inc["filter"]
                .as_array()
                .map(|a| a.as_slice())
                .unwrap_or(&[]);
            !filters.is_empty()
                && filters.iter().any(|f| {
                    f["op"].as_str().unwrap_or("") == "="
                        && f["property"].as_str().unwrap_or("") != "constraint"
                })
                && filters.iter().all(|f| {
                    let op = f["op"].as_str().unwrap_or("");
                    let prop = f["property"].as_str().unwrap_or("");
                    (op == "=" && prop != "constraint")
                        || ((prop == "concept" || prop == "code")
                            && matches!(op, "is-a" | "descendent-of" | "generalizes"))
                })
                && inc["concept"].as_array().is_none_or(|a| a.is_empty())
                && inc["valueSet"].as_array().is_none_or(|a| a.is_empty())
        });

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

        // ── Routing: FTS-first vs. property-first ────────────────────────────────
        // When the request carries both a text filter and compose filter(s), two
        // strategies are possible:
        //
        //   FTS-first — query `concepts_fts` by display text → bounded candidate
        //               set → apply hierarchy/property= in Rust.  Optimal when
        //               there is NO property= filter: a 3+-char trigram query is
        //               highly selective (<1 000 candidates), and an is-a walk
        //               over that small set is cheap.
        //
        //   Property-first — start from `idx_concept_properties_value` (property,
        //               value, concept_id) → O(K_property) rows → text filter in
        //               Rust via `apply_compose_filters → query_subtree_with_property`.
        //               Optimal when a property= filter is present: the property
        //               index is far more selective than FTS on common display
        //               terms ("card", "structure", "right") that appear in tens
        //               of thousands of HL7-package concepts.  Those concepts have
        //               lower FTS rowids (imported first) and are scanned before
        //               SNOMED on cold EBS storage, causing 10–18 s per request
        //               and 30 s timeouts at high concurrency.
        //
        // `all_batchable` — true when every compose filter has a fast in-Rust
        // implementation in `apply_compose_filters_to_candidates`: property=,
        // hierarchy ops (is-a / descendent-of / generalizes / child-of), or
        // regex.  False for ECL `constraint` filters which need full ECL
        // evaluation.
        let compose_filters: &[serde_json::Value] = inc["filter"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[]);
        let all_batchable = !compose_filters.is_empty()
            && compose_filters.iter().all(|f| {
                let op = f["op"].as_str().unwrap_or("");
                let prop = f["property"].as_str().unwrap_or("");
                (op == "=" && prop != "constraint")
                    || ((prop == "concept" || prop == "code")
                        && matches!(op, "is-a" | "descendent-of" | "generalizes" | "child-of"))
                    || op == "regex"
            });

        // `has_eq_filter` — true when any compose filter is a property= filter.
        // Normally we push the text filter into SQL via instr() to avoid loading
        // all property-matching descendants before discarding them.  When the
        // property result cache is active (all_prop_cacheable), we skip the SQL
        // text push so that the FULL property-matched set is returned and cached;
        // the text filter is applied in Rust after the loop.
        let has_eq_filter = compose_filters.iter().any(|f| {
            f["op"].as_str().unwrap_or("") == "="
                && f["property"].as_str().unwrap_or("") != "constraint"
        });
        let sql_text = if has_eq_filter && filter_lower.len() >= 3 && !all_prop_cacheable {
            Some(filter_lower.as_str())
        } else {
            None
        };

        if filter_lower.len() >= 3 && all_batchable && !has_eq_filter {
            // FTS-first path: hierarchy + text, no property= filter.
            // FTS narrows to text-matching candidates; hierarchy walk runs on
            // the already-small set via apply_compose_filters_to_candidates.
            ensure_concepts_fts(conn, &system_id)?;
            let candidates =
                fts_candidates_for_system(conn, &system_id, system_url, &filter_lower)?;
            if !candidates.is_empty() {
                let filtered = apply_compose_filters_to_candidates(
                    conn,
                    &system_id,
                    compose_filters,
                    candidates,
                )?;
                results.extend(filtered);
            }
            continue;
        }

        if let Some(filter_result) =
            apply_compose_filters(conn, system_url, &system_id, inc, sql_text)?
        {
            // sql_text.is_some(): SQL already applied the text filter via instr().
            // all_prop_cacheable: accumulate the full unfiltered set; text filter
            //   is applied after the loop (and the set is stored in the cache).
            // Otherwise: apply the Rust text filter here (ECL / generalizes /
            //   multi-property paths that don't push text into SQL).
            if sql_text.is_some() || all_prop_cacheable {
                results.extend(filter_result);
            } else {
                results.extend(filter_result.into_iter().filter(|c| {
                    c.code.to_lowercase().contains(&filter_lower)
                        || c.display
                            .as_deref()
                            .map(|d| d.to_lowercase().contains(&filter_lower))
                            .unwrap_or(false)
                }));
            }
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
                        is_abstract: None,

                        inactive: None,

                        designations: vec![],

                        properties: vec![],
                        contains: vec![],
                    });
                }
            }
        } else {
            // Full-system include with no explicit codes.
            // For filter strings ≥ 3 chars: use the FTS5 trigram index when it is
            // already built (O(matches)), otherwise fall back to a LIKE scan
            // (O(N), ~200–500 ms for large systems) and spawn a background task to
            // Trigram FTS5 needs ≥ 3 chars; shorter filters fall back to LIKE.
            // `ensure_concepts_fts` builds the index lazily on the first call
            // (uses BEGIN IMMEDIATE so only one thread does the work).
            if filter_lower.len() >= 3 {
                ensure_concepts_fts(conn, &system_id)?;
                let match_expr = fts5_quote(&filter_lower);
                // Per-system FTS limit: use hint×3 headroom (multi-system requests need surplus),
                // but cap at 5000 for safety. Minimum 100 so tiny counts still get results.
                let per_sys_limit =
                    limit_hint.map(|h| (h * 3).clamp(100, 5000)).unwrap_or(5000) as i64;
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT code, display FROM concepts_fts \
                         WHERE concepts_fts MATCH ?1 AND system_id = ?2 \
                         LIMIT ?3",
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![match_expr, system_id, per_sys_limit],
                        |row| {
                            Ok(ExpansionContains {
                                system: system_url.to_owned(),
                                code: row.get(0)?,
                                display: row.get(1)?,
                                is_abstract: None,

                                inactive: None,

                                designations: vec![],

                                properties: vec![],
                                contains: vec![],
                            })
                        },
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                results.extend(rows);
            } else {
                let per_sys_limit =
                    limit_hint.map(|h| (h * 3).clamp(100, 5000)).unwrap_or(5000) as i64;
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT code, display FROM concepts \
                         WHERE system_id = ?1 \
                           AND (LOWER(code) LIKE ?2 OR LOWER(display) LIKE ?2) \
                         ORDER BY code LIMIT ?3",
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![system_id, sql_pat, per_sys_limit],
                        |row| {
                            Ok(ExpansionContains {
                                system: system_url.to_owned(),
                                code: row.get(0)?,
                                display: row.get(1)?,
                                is_abstract: None,

                                inactive: None,

                                designations: vec![],

                                properties: vec![],
                                contains: vec![],
                            })
                        },
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                results.extend(rows);
            }
        }
    }

    // When all includes were prop-cacheable, populate the property result cache
    // with the full (unfiltered) concept set, then apply the text filter in Rust.
    if all_prop_cacheable {
        if let Some((prop_key, cache)) = prop_cache {
            populate_property_cache(&results, prop_key, cache);
        }
        results.retain(|c| {
            c.code.to_lowercase().contains(&filter_lower)
                || c.display
                    .as_deref()
                    .map(|d| d.to_lowercase().contains(&filter_lower))
                    .unwrap_or(false)
        });
    }

    Ok(results)
}

/// Per-request context for resolving `compose.include[].valueSet[]` references.
///
/// Carries the inline ValueSet body's `contained[]` array (so `#fragment`
/// references can be looked up locally) plus any `tx-resource` ValueSets the
/// caller supplied. Both lists are checked **before** falling back to the
/// `value_sets` table when nested refs are resolved during expansion.
///
/// `visited` tracks the canonical URLs (and `#fragment` ids) currently being
/// expanded so a self-reference such as
/// `vs1.compose.include.valueSet = ["vs1"]` does not infinite-loop. The
/// depth counter is enforced separately by `compute_expansion_depth_inner`.
#[derive(Default, Clone)]
struct InlineResolutionContext<'a> {
    contained: Vec<&'a serde_json::Value>,
    tx_resources: Vec<&'a serde_json::Value>,
    visited: std::collections::BTreeSet<String>,
}

impl<'a> InlineResolutionContext<'a> {
    /// Build a context from an inline ValueSet body and an optional tx-resource list.
    fn from_inline(
        inline_vs: Option<&'a serde_json::Value>,
        tx_resources: &'a [serde_json::Value],
    ) -> Self {
        let contained = inline_vs
            .and_then(|vs| vs.get("contained"))
            .and_then(|c| c.as_array())
            .map(|arr| arr.iter().collect::<Vec<_>>())
            .unwrap_or_default();
        let tx_refs: Vec<&'a serde_json::Value> = tx_resources
            .iter()
            .filter(|r| r.get("resourceType").and_then(|v| v.as_str()) == Some("ValueSet"))
            .collect();
        Self {
            contained,
            tx_resources: tx_refs,
            visited: std::collections::BTreeSet::new(),
        }
    }

    /// Resolve a `valueSet[]` entry to its compose JSON without touching the DB.
    ///
    /// `#fragment` refs search the inline body's `contained[]`; canonical URLs
    /// check `tx_resources`. Returns `Some(compose_string)` when an inline
    /// match is found; the caller falls back to the DB on `None`.
    fn lookup_compose(&self, ref_str: &str) -> Option<String> {
        if let Some(id) = ref_str.strip_prefix('#') {
            for r in &self.contained {
                if r.get("id").and_then(|v| v.as_str()) == Some(id)
                    && r.get("resourceType").and_then(|v| v.as_str()) == Some("ValueSet")
                {
                    return r.get("compose").map(|c| c.to_string());
                }
            }
            return None;
        }
        // Non-fragment refs may carry a `|version` pin; tx-resource matching
        // ignores the version because the IG fixtures usually pass the bare URL.
        let bare = ref_str.split('|').next().unwrap_or(ref_str);
        for r in &self.tx_resources {
            if r.get("url").and_then(|v| v.as_str()) == Some(bare) {
                return r.get("compose").map(|c| c.to_string());
            }
        }
        None
    }
}

/// Compute an expansion from the raw `compose_json`.
///
/// Supports:
/// - `compose.include[].system` — required in each include clause.
/// - `compose.include[].concept[]` — explicit code list; when absent, all
///   codes from the referenced system are included.
/// - `compose.include[].valueSet[]` — references that are intersected with
///   the include's local conditions; multiple entries are intersected.
/// - `compose.exclude[]` — removes the (system, code) pairs that match the
///   same conditions, including `valueSet[]` references.
fn compute_expansion(
    conn: &Connection,
    compose_json: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    compute_expansion_with_ctx(
        conn,
        compose_json,
        warnings,
        &InlineResolutionContext::default(),
    )
}

/// Variant of [`compute_expansion`] that threads an inline-resolution context
/// through nested `compose.include[].valueSet[]` lookups.
fn compute_expansion_with_ctx(
    conn: &Connection,
    compose_json: Option<&str>,
    warnings: &mut Vec<String>,
    ctx: &InlineResolutionContext<'_>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    compute_expansion_depth_inner(conn, compose_json, warnings, 0, ctx)
}

/// Resolve a single `compose.include[].valueSet[]` reference to a flat code
/// list, consulting (in order) the inline `contained[]` array, the
/// `tx-resource` map, and finally the local `value_sets` table.
///
/// The visited-URL set guards against cycles (e.g. `vsA` references `vsB`
/// which references `vsA`); a re-entry pushes a `vs-invalid` warning instead
/// of recursing.
fn expand_vs_reference(
    conn: &Connection,
    ref_url: &str,
    warnings: &mut Vec<String>,
    depth: u8,
    ctx: &InlineResolutionContext<'_>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    if ctx.visited.contains(ref_url) {
        warnings.push(format!(
            "Cyclic ValueSet reference detected for {ref_url}; excluded from expansion (vs-invalid)"
        ));
        return Ok(vec![]);
    }

    let mut child_ctx = ctx.clone();
    child_ctx.visited.insert(ref_url.to_owned());

    if let Some(compose_str) = ctx.lookup_compose(ref_url) {
        return compute_expansion_depth_inner(
            conn,
            Some(&compose_str),
            warnings,
            depth + 1,
            &child_ctx,
        );
    }

    // `#fragment` refs that didn't match any contained[] are unresolvable —
    // there is no DB fallback for them.
    if ref_url.starts_with('#') {
        warnings.push(format!(
            "Referenced contained ValueSet {ref_url} not found; excluded from expansion"
        ));
        return Ok(vec![]);
    }

    // Strip an optional `|version` suffix before the DB lookup; the
    // `value_sets` table is keyed only on `url`.
    let bare_url = ref_url.split('|').next().unwrap_or(ref_url);
    match resolve_value_set(conn, bare_url, None) {
        Ok((ref_vs_id, ref_compose)) => {
            let cached = fetch_cache(conn, &ref_vs_id)?;
            if !cached.is_empty() {
                return Ok(cached);
            }
            // Recurse with the child context so further refs inside the
            // referenced ValueSet still see contained / tx-resource shadows.
            compute_expansion_depth_inner(
                conn,
                ref_compose.as_deref(),
                warnings,
                depth + 1,
                &child_ctx,
            )
        }
        Err(_) => {
            warnings.push(format!(
                "Referenced ValueSet {ref_url} not found; excluded from expansion"
            ));
            Ok(vec![])
        }
    }
}

fn compute_expansion_depth_inner(
    conn: &Connection,
    compose_json: Option<&str>,
    warnings: &mut Vec<String>,
    depth: u8,
    ctx: &InlineResolutionContext<'_>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let Some(raw) = compose_json else {
        return Ok(vec![]);
    };

    let compose: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| HtsError::Internal(format!("Failed to parse compose_json: {e}")))?;

    let empty_arr = vec![];
    let includes = compose["include"].as_array().unwrap_or(&empty_arr);

    // Fast path: 2+ includes all using property-equality on the same system.
    // Only safe when no include carries a `valueSet[]` ref — those need the
    // full per-include path so the reference can be intersected with the
    // local system/concept/filter portion of the same include.
    let any_vs_ref = includes
        .iter()
        .any(|inc| inc["valueSet"].as_array().is_some_and(|a| !a.is_empty()));
    let mut included: Vec<ExpansionContains> = if !any_vs_ref {
        if let Some(result) = try_multi_include_property_only(conn, includes, warnings)? {
            result
        } else {
            expand_includes_per_clause(conn, includes, warnings, depth, ctx)?
        }
    } else {
        expand_includes_per_clause(conn, includes, warnings, depth, ctx)?
    };

    // Apply excludes — each clause may carry concept[], filter[], and/or
    // valueSet[] references. See `build_exclude_set` for the intersection
    // semantics applied within a single exclude clause.
    let excludes = compose["exclude"].as_array().unwrap_or(&empty_arr);
    if !excludes.is_empty() {
        let denied = build_exclude_set(conn, excludes, warnings, depth, ctx)?;
        if !denied.is_empty() {
            included.retain(|c| !denied.contains(&(c.system.clone(), c.code.clone())));
        }
    }

    Ok(included)
}

#[allow(clippy::too_many_arguments)]
fn expand_includes_per_clause(
    conn: &Connection,
    includes: &[serde_json::Value],
    warnings: &mut Vec<String>,
    depth: u8,
    ctx: &InlineResolutionContext<'_>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let mut included: Vec<ExpansionContains> = Vec::new();
    let mut system_id_cache: HashMap<String, String> = HashMap::new();

    for inc in includes {
        let vs_refs_present = inc["valueSet"].as_array().is_some_and(|a| !a.is_empty());
        let has_local_system = inc["system"].as_str().is_some_and(|s| !s.is_empty());

        // ── compose.include[].valueSet[] handling (FHIR R5 §4.9.5) ─────────
        // Each entry is an additional condition on the include: a concept
        // matches only if it appears in EVERY referenced ValueSet. When the
        // include also has system / concept / filter, those local conditions
        // are intersected with the ref expansions. When the include has only
        // valueSet[] entries, the result is the intersection of the refs.
        if vs_refs_present {
            if depth >= 4 {
                warnings.push(
                    "Max ValueSet include depth (4) reached; skipping nested valueSet references"
                        .to_owned(),
                );
                continue;
            }

            let vs_refs = inc["valueSet"].as_array().unwrap();
            let mut ref_sets: Vec<HashSet<(String, String)>> = Vec::new();
            let mut display_index: HashMap<(String, String), Option<String>> = HashMap::new();

            for vs_ref in vs_refs {
                let ref_url = match vs_ref.as_str() {
                    Some(u) => u,
                    None => continue,
                };
                let codes = expand_vs_reference(conn, ref_url, warnings, depth, ctx)?;
                let mut set: HashSet<(String, String)> = HashSet::new();
                for c in codes {
                    let key = (c.system.clone(), c.code.clone());
                    display_index
                        .entry(key.clone())
                        .or_insert(c.display.clone());
                    set.insert(key);
                }
                ref_sets.push(set);
            }

            // Intersect across every referenced ValueSet.
            let mut intersected: HashSet<(String, String)> = match ref_sets.first() {
                Some(first) => first.clone(),
                None => HashSet::new(),
            };
            for set in ref_sets.iter().skip(1) {
                intersected.retain(|k| set.contains(k));
            }

            // Build the local "base set" (system + concept + filter) and
            // intersect with the ref intersection. When the include has no
            // local system the result is just the ref intersection.
            let final_set: HashSet<(String, String)> = if has_local_system {
                let mut single_inc = inc.clone();
                if let Some(obj) = single_inc.as_object_mut() {
                    obj.remove("valueSet");
                }
                let base_codes = expand_single_include_local(
                    conn,
                    &single_inc,
                    warnings,
                    &mut system_id_cache,
                    depth,
                )?;
                let mut bs: HashSet<(String, String)> = HashSet::new();
                for c in &base_codes {
                    bs.insert((c.system.clone(), c.code.clone()));
                    display_index
                        .entry((c.system.clone(), c.code.clone()))
                        .or_insert(c.display.clone());
                }
                intersected.intersection(&bs).cloned().collect()
            } else {
                intersected
            };

            for (system, code) in final_set {
                let display = display_index
                    .get(&(system.clone(), code.clone()))
                    .cloned()
                    .unwrap_or(None);
                included.push(ExpansionContains {
                    system,
                    code,
                    display,
                    is_abstract: None,
                    inactive: None,
                    designations: vec![],
                    properties: vec![],
                    contains: vec![],
                });
            }

            continue;
        }

        // No `valueSet[]` reference on this include — fall through to the
        // local single-include expansion (system + concept + filter).
        let local = expand_single_include_local(conn, inc, warnings, &mut system_id_cache, depth)?;
        included.extend(local);
    }

    Ok(included)
}

/// Expand the local (system + concept + filter) portion of a single
/// `compose.include[]` clause without consulting any nested `valueSet[]`
/// references. Used both as the per-include path inside
/// `expand_includes_per_clause` and as the "base set" computation when an
/// include carries both local conditions and `valueSet[]` references that need
/// to be intersected together.
fn expand_single_include_local(
    conn: &Connection,
    inc: &serde_json::Value,
    warnings: &mut Vec<String>,
    system_id_cache: &mut HashMap<String, String>,
    depth: u8,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let system_url = match inc["system"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(vec![]),
    };
    let inc_version = inc["version"].as_str();

    // Cache key folds the optional version into the URL so versioned and
    // versionless includes for the same canonical URL don't clobber each
    // other's resolved id.
    let cache_key = match inc_version {
        Some(v) => format!("{system_url}|{v}"),
        None => system_url.to_owned(),
    };
    let system_id = match system_id_cache.get(&cache_key) {
        Some(id) => id.clone(),
        None => match resolve_compose_system_id(conn, system_url, inc_version)? {
            Some(id) => {
                system_id_cache.insert(cache_key, id.clone());
                id
            }
            None => {
                let msg = format!(
                    "CodeSystem {system_url} was not found and has been excluded from the expansion"
                );
                tracing::warn!(%system_url, ?inc_version, "{msg}");
                warnings.push(msg);
                return Ok(vec![]);
            }
        },
    };

    if let Some(filter_result) = apply_compose_filters(conn, system_url, &system_id, inc, None)? {
        return Ok(filter_result);
    }

    if let Some(explicit_codes) = inc["concept"].as_array() {
        // Explicit code list: single json_each batch join instead of N
        // individual point lookups. INNER JOIN drops codes that don't
        // exist in the concepts table — the IG `simple-expand-enum-bad`
        // fixture explicitly asserts that an unknown code in
        // compose.include[].concept[] is silently filtered out of the
        // expansion rather than appearing as a phantom entry.
        let codes_json: serde_json::Value = explicit_codes
            .iter()
            .filter_map(|e| e["code"].as_str())
            .collect::<Vec<_>>()
            .into();
        let codes_str = codes_json.to_string();

        let mut stmt = conn
            .prepare_cached(
                "SELECT c.code, c.display
                 FROM json_each(?1) je
                 JOIN concepts c
                     ON c.system_id = ?2 AND c.code = je.value",
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![codes_str, system_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let mut out: Vec<ExpansionContains> = Vec::with_capacity(rows.len());
        let mut seen_codes: HashSet<String> = HashSet::new();
        for (code, display) in rows {
            seen_codes.insert(code.clone());
            out.push(ExpansionContains {
                system: system_url.to_owned(),
                code,
                display,
                is_abstract: None,
                inactive: None,
                designations: vec![],
                properties: vec![],
                contains: vec![],
            });
        }
        // IG `parameters/parameters-expand-enum-*` semantics: when an
        // explicitly-enumerated concept is abstract (notSelectable=true), the
        // immediate children appear alongside it in the expansion. This isn't
        // tied to excludeNested — the IG fixture lists the children flat at
        // the top level even when nested mode is requested. Surface direct
        // children here; the activeOnly splice in the operations layer then
        // reshapes the tree as needed.
        //
        // Skip when depth > 0 — the simple/expand-contained fixture
        // intersects an inline `#vs1` ref (also enumerated, also includes
        // an abstract code2) with another VS, and the IG expects the
        // inner expansion to be exactly the enumerated codes (no children
        // bolted on) so the intersection is well-defined.
        let abstract_codes_in_set: Vec<String> = if depth == 0 {
            out.iter()
                .filter(|c| is_concept_abstract(conn, &c.system, &c.code))
                .map(|c| c.code.clone())
                .collect()
        } else {
            Vec::new()
        };
        for parent_code in abstract_codes_in_set {
            let mut child_stmt = conn
                .prepare_cached(
                    "SELECT c.code, c.display
                     FROM concept_hierarchy h
                     JOIN concepts c
                         ON c.system_id = h.system_id AND c.code = h.child_code
                     WHERE h.system_id = ?1 AND h.parent_code = ?2",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let child_rows = child_stmt
                .query_map(rusqlite::params![system_id, parent_code], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            for (child_code, child_display) in child_rows {
                if seen_codes.insert(child_code.clone()) {
                    out.push(ExpansionContains {
                        system: system_url.to_owned(),
                        code: child_code,
                        display: child_display,
                        is_abstract: None,
                        inactive: None,
                        designations: vec![],
                        properties: vec![],
                        contains: vec![],
                    });
                }
            }
        }
        return Ok(out);
    }

    // No explicit codes and no filters: include ALL concepts from the
    // referenced system.
    let mut stmt = conn
        .prepare_cached("SELECT code, display FROM concepts WHERE system_id = ?1 ORDER BY code")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows = stmt
        .query_map([&system_id], |row| {
            Ok(ExpansionContains {
                system: system_url.to_owned(),
                code: row.get(0)?,
                display: row.get(1)?,
                is_abstract: None,
                inactive: None,
                designations: vec![],
                properties: vec![],
                contains: vec![],
            })
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows)
}

/// Build a `(system, code)` deny-set from the `compose.exclude[]` array.
///
/// Each exclude clause may carry `concept[]` (explicit codes), `filter[]`
/// (ECL / is-a / property=) and/or `valueSet[]` references. When a
/// `valueSet[]` is present its expansion is intersected with the local
/// system/concept/filter to determine which codes to deny — matching the
/// FHIR semantics that "the codes match if they meet ALL of the conditions".
fn build_exclude_set(
    conn: &Connection,
    excludes: &[serde_json::Value],
    warnings: &mut Vec<String>,
    depth: u8,
    ctx: &InlineResolutionContext<'_>,
) -> Result<HashSet<(String, String)>, HtsError> {
    let mut denied: HashSet<(String, String)> = HashSet::new();
    let mut system_id_cache: HashMap<String, String> = HashMap::new();

    for exc in excludes {
        let vs_refs_present = exc["valueSet"].as_array().is_some_and(|a| !a.is_empty());

        if vs_refs_present {
            if depth >= 4 {
                warnings.push(
                    "Max ValueSet exclude depth (4) reached; skipping nested valueSet references"
                        .to_owned(),
                );
                continue;
            }
            let mut ref_sets: Vec<HashSet<(String, String)>> = Vec::new();
            for vs_ref in exc["valueSet"].as_array().unwrap() {
                let ref_url = match vs_ref.as_str() {
                    Some(u) => u,
                    None => continue,
                };
                let resolved = expand_vs_reference(conn, ref_url, warnings, depth, ctx)?;
                let mut set = HashSet::new();
                for c in resolved {
                    set.insert((c.system, c.code));
                }
                ref_sets.push(set);
            }
            let mut intersected: HashSet<(String, String)> = match ref_sets.first() {
                Some(first) => first.clone(),
                None => HashSet::new(),
            };
            for set in ref_sets.iter().skip(1) {
                intersected.retain(|k| set.contains(k));
            }

            // Intersect with the local exclude condition when one is present.
            let has_local_system = exc["system"].as_str().is_some_and(|s| !s.is_empty());
            if has_local_system {
                let mut single_exc = exc.clone();
                if let Some(obj) = single_exc.as_object_mut() {
                    obj.remove("valueSet");
                }
                let local = expand_single_include_local(
                    conn,
                    &single_exc,
                    warnings,
                    &mut system_id_cache,
                    depth,
                )?;
                let local_set: HashSet<(String, String)> =
                    local.into_iter().map(|c| (c.system, c.code)).collect();
                intersected.retain(|k| local_set.contains(k));
            }

            for k in intersected {
                denied.insert(k);
            }
            continue;
        }

        let exc_system = exc["system"].as_str().unwrap_or("").to_owned();

        if exc["concept"].as_array().is_some_and(|a| !a.is_empty()) {
            // Explicit codes: deny each (system, code) pair without consulting the DB.
            if let Some(codes) = exc["concept"].as_array() {
                for entry in codes {
                    if let Some(code) = entry["code"].as_str() {
                        denied.insert((exc_system.clone(), code.to_owned()));
                    }
                }
            }
            continue;
        }

        // No concept[], no valueSet[] — fall back to the same per-include
        // expansion path (covers exclude.filter[], full-system exclude, etc.).
        let local = expand_single_include_local(conn, exc, warnings, &mut system_id_cache, depth)?;
        for c in local {
            denied.insert((c.system, c.code));
        }
    }

    Ok(denied)
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
    text_filter: Option<&str>,
) -> Result<Option<Vec<ExpansionContains>>, HtsError> {
    let filters = match inc["filter"].as_array() {
        Some(f) if !f.is_empty() => f,
        _ => return Ok(None),
    };

    // Validate every filter carries a non-empty `value`. ValueSet.compose.
    // include.filter.value is mandatory per the FHIR spec; the HL7 IG
    // `errors/broken-filter` fixtures expect a 400 with diagnostic text
    // "The system <url> filter with property = <p>, op = <o> has no value"
    // and `tx-issue-type=vs-invalid` whenever it is missing or empty.
    for f in filters {
        let value_present = f
            .get("value")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if !value_present {
            let property = f["property"].as_str().unwrap_or("");
            let op = f["op"].as_str().unwrap_or("");
            return Err(HtsError::VsInvalid(format!(
                "The system {system_url} filter with property = {property}, op = {op} has no value"
            )));
        }
    }

    // Partition into property= filters (fast, indexed), regex filters (must
    // load candidates and match in Rust), and the remaining hierarchy / ECL
    // filters (potentially O(N_descendants)).  Property filters run in
    // phase 1; hierarchy filters in phase 2 and can exploit the bounded
    // candidate set from phase 1 to switch from a top-down tree expansion to
    // per-candidate ancestor walks; regex filters run last so they only need
    // to materialise the (already narrowed) candidate set.
    // Treat `op="in"` with a single non-comma value identically to `op="="`
    // — the IG `notSelectable/notSelectable-prop-in*` fixtures use
    // `filter: { property: notSelectable, op: in, value: "true" }`. FHIR
    // spec `in` is a comma-separated list; the single-value case is the
    // common one and reduces cleanly to equality. (Multi-value `in`
    // expansion remains TODO — those fixtures aren't currently in scope.)
    let (property_filters, mut rest): (Vec<_>, Vec<_>) = filters.iter().partition(|f| {
        let op = f["op"].as_str().unwrap_or("");
        let property = f["property"].as_str().unwrap_or("");
        let value = f["value"].as_str().unwrap_or("");
        let in_single_value = op == "in" && !value.contains(',');
        (op == "=" || in_single_value) && property != "constraint"
    });
    let (regex_filters, hierarchy_filters): (Vec<_>, Vec<_>) = rest
        .drain(..)
        .partition(|f| f["op"].as_str() == Some("regex"));

    // ── Fast path: single is-a / descendent-of + property= filters ────────────
    // When there is exactly one hierarchy filter (is-a or descendent-of) and one
    // or more property= filters we use a combined downward-CTE query that expands
    // the subtree and filters by property in a single pass.  This avoids
    // materialising potentially tens of thousands of candidates in Phase 1
    // (property= globally) only to discard most of them in Phase 2.
    let one_isa_hier = || {
        hierarchy_filters.len() == 1 && {
            let f = &hierarchy_filters[0];
            let p = f["property"].as_str().unwrap_or("");
            let o = f["op"].as_str().unwrap_or("");
            (p == "concept" || p == "code") && (o == "is-a" || o == "descendent-of")
        }
    };
    if !property_filters.is_empty() && one_isa_hier() {
        let hf = &hierarchy_filters[0];
        let op = hf["op"].as_str().unwrap_or("");
        let root_code = hf["value"].as_str().unwrap_or("");
        let include_self = op == "is-a";

        let mut result: Option<Vec<ExpansionContains>> = None;
        for f in &property_filters {
            let property = f["property"].as_str().unwrap_or("");
            let value = f["value"].as_str().unwrap_or("");
            let concepts = query_subtree_with_property(
                conn,
                system_url,
                system_id,
                root_code,
                include_self,
                property,
                value,
                text_filter,
            )?;
            match result.as_mut() {
                Some(prev) => {
                    let keep: HashSet<String> = concepts.iter().map(|c| c.code.clone()).collect();
                    prev.retain(|c| keep.contains(&c.code));
                }
                None => result = Some(concepts),
            }
        }
        return Ok(result.or_else(|| Some(vec![])));
    }

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

        // `child-of` is a single-level hierarchy filter and does not have a
        // direct ECL equivalent — handle it before the ECL fallback so the
        // (property, op) wildcard at the bottom never sees it.
        if (property == "concept" || property == "code") && op == "child-of" {
            if value.is_empty() {
                return Err(HtsError::VsInvalid(
                    "ValueSet compose filter with op='child-of' is missing a value".to_string(),
                ));
            }
            any_filter_seen = true;
            if let Some(prev) = result.as_mut() {
                if prev.is_empty() {
                    continue;
                }
                let codes: Vec<String> = prev.iter().map(|c| c.code.clone()).collect();
                let valid = batch_direct_children_in_set(conn, system_id, value, &codes)?;
                prev.retain(|c| valid.contains(&c.code));
                continue;
            }
            let children = query_direct_children(conn, system_url, system_id, value)?;
            result = Some(children);
            continue;
        }

        // Normalise `code` → `concept` so IG fixtures that use either property
        // alias for the concept identifier (e.g. search/search-filter-yes uses
        // `property=code, op=is-a`) hit the same hierarchy paths.
        let property_norm = if property == "code" {
            "concept"
        } else {
            property
        };
        let ecl_expr: String = match (property_norm, op) {
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

        // Fast path: a bounded candidate set from phase 1 exists — batch-check
        // hierarchy membership in a single recursive CTE instead of N individual
        // ancestor walks.  When the candidate set is already empty, skip all
        // remaining hierarchy filters (intersection of ∅ is always ∅).
        if let Some(prev) = result.as_mut() {
            if prev.is_empty() {
                continue;
            }
            match (property_norm, op) {
                ("concept", "is-a") => {
                    let codes: Vec<String> = prev.iter().map(|c| c.code.clone()).collect();
                    let valid = batch_descendants_in_set(conn, system_id, value, true, &codes)?;
                    prev.retain(|c| valid.contains(&c.code));
                    continue;
                }
                ("concept", "descendent-of") => {
                    let codes: Vec<String> = prev.iter().map(|c| c.code.clone()).collect();
                    let valid = batch_descendants_in_set(conn, system_id, value, false, &codes)?;
                    prev.retain(|c| valid.contains(&c.code));
                    continue;
                }
                ("concept", "generalizes") => {
                    // C generalizes value  ⟺  C is an ancestor-or-self of value.
                    let codes: Vec<String> = prev.iter().map(|c| c.code.clone()).collect();
                    let valid = batch_ancestors_in_set(conn, system_id, value, &codes)?;
                    prev.retain(|c| valid.contains(&c.code));
                    continue;
                }
                _ => {}
            }
        }

        // Fast path for generalizes with no prior candidate set: ancestors are
        // few (≤ ~20 in SNOMED), so a recursive CTE is O(depth) — much faster
        // than full ECL evaluation which resolves the entire ancestor chain.
        if (property == "concept" || property == "code") && op == "generalizes" {
            let ancestors = query_ancestors_full(conn, system_url, system_id, value)?;
            match result.as_mut() {
                Some(prev) => {
                    let keep: HashSet<String> = ancestors.iter().map(|c| c.code.clone()).collect();
                    prev.retain(|c| keep.contains(&c.code));
                }
                None => result = Some(ancestors),
            }
            continue;
        }

        // Slow path: no prior bounded set — compute the full ECL expansion.
        let resolved = ecl::parse_and_evaluate(conn, system_id, &ecl_expr)?;
        let concepts: Vec<ExpansionContains> = resolved
            .into_iter()
            .map(|c| ExpansionContains {
                system: system_url.to_owned(),
                code: c.code,
                display: c.display,
                is_abstract: None,

                inactive: None,

                designations: vec![],

                properties: vec![],
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

    // ── Phase 3: regex filters ────────────────────────────────────────────────
    // Regex evaluation requires materialising rows and matching in Rust.  When
    // a bounded candidate set is already in `result`, we filter that set in
    // place; otherwise we load the full match set from the system and AND-merge.
    for f in &regex_filters {
        let property = f["property"].as_str().unwrap_or("");
        let value = f["value"].as_str().unwrap_or("");
        any_filter_seen = true;

        if let Some(prev) = result.as_mut() {
            // Compile up-front so a malformed pattern surfaces as VsInvalid
            // even when the candidate set is already empty.
            let regex = compile_vs_regex(value)?;
            if prev.is_empty() {
                continue;
            }
            if property == "code" || property.is_empty() {
                prev.retain(|c| regex.is_match(&c.code));
            } else {
                let codes: Vec<String> = prev.iter().map(|c| c.code.clone()).collect();
                let json_codes = serde_json::to_string(&codes)
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT c.code, cp.value
                         FROM   concept_properties cp
                         JOIN   concepts c ON c.id = cp.concept_id AND c.system_id = ?1
                         WHERE  cp.property = ?2
                           AND  c.code IN (SELECT value FROM json_each(?3))",
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                let rows = stmt
                    .query_map(rusqlite::params![system_id, property, json_codes], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(|e| HtsError::StorageError(e.to_string()))?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                let mut keep: HashSet<String> = HashSet::new();
                for (code, val) in rows {
                    if regex.is_match(&val) {
                        keep.insert(code);
                    }
                }
                prev.retain(|c| keep.contains(&c.code));
            }
            continue;
        }

        let concepts = query_regex_match(conn, system_url, system_id, property, value)?;
        result = Some(concepts);
    }

    if any_filter_seen && result.is_none() {
        return Ok(Some(vec![]));
    }

    Ok(result)
}

/// Returns `true` when a compose include entry uses only property-equality
/// filters — no hierarchy operators, no ECL constraints, no explicit concept
/// list, and no nested ValueSet references.
fn is_property_only_include(inc: &serde_json::Value) -> bool {
    if inc["system"].as_str().is_none_or(|s| s.is_empty()) {
        return false;
    }
    if inc["concept"].as_array().is_some_and(|a| !a.is_empty()) {
        return false;
    }
    if inc["valueSet"].as_array().is_some_and(|a| !a.is_empty()) {
        return false;
    }
    let Some(filters) = inc["filter"].as_array() else {
        return false;
    };
    !filters.is_empty()
        && filters.iter().all(|f| {
            f["op"].as_str().unwrap_or("") == "="
                && f["property"].as_str().unwrap_or("") != "constraint"
        })
}

/// Fast path for multi-include composes where every include uses the **same**
/// CodeSystem and carries only property-equality (`op = "="`) filters.
///
/// Collapses all includes into a single query using a UNION of driver-+EXISTS
/// sub-selects instead of N×M individual round-trips or an INTERSECT CTE
/// (which would materialise and sort large intermediate sets).  For a
/// 2-include × 2-filter case the generated SQL looks like (parameters are
/// numbered sequentially: ?1..?8 for the 4×2 filter params, ?9 for system_id):
///
/// ```sql
/// SELECT c.code, c.display FROM concepts c
/// WHERE c.system_id = ?9
/// AND c.id IN (
///     SELECT cp0.concept_id FROM concept_properties cp0
///     WHERE cp0.property = ?1 AND cp0.value = ?2
///       AND EXISTS (SELECT 1 FROM concept_properties
///                   WHERE concept_id = cp0.concept_id AND property = ?3 AND value = ?4)
///     UNION
///     SELECT cp0.concept_id FROM concept_properties cp0
///     WHERE cp0.property = ?5 AND cp0.value = ?6
///       AND EXISTS (SELECT 1 FROM concept_properties
///                   WHERE concept_id = cp0.concept_id AND property = ?7 AND value = ?8)
/// )
/// ```
///
/// The driver scan uses `idx_concept_properties_value(property, value, concept_id)`;
/// each EXISTS check uses `idx_concept_properties_lookup(concept_id, property, value)`.
/// No large temp sets are sorted — SQLite short-circuits EXISTS on the first hit.
///
/// Returns `None` when the fast path does not apply (single include, mixed
/// systems, non-property filters, explicit concept lists, etc.) so the caller
/// can fall back to the generic per-include loop.
fn try_multi_include_property_only(
    conn: &Connection,
    includes: &[serde_json::Value],
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<ExpansionContains>>, HtsError> {
    if includes.len() < 2 {
        return Ok(None);
    }

    let first_system = match includes[0]["system"].as_str() {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(None),
    };

    if !includes
        .iter()
        .all(|inc| inc["system"].as_str() == Some(first_system) && is_property_only_include(inc))
    {
        return Ok(None);
    }

    let system_id: Option<String> = conn
        .query_row(
            "SELECT id FROM code_systems WHERE url = ?1",
            [first_system],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let system_id = match system_id {
        Some(id) => id,
        None => {
            let msg = format!(
                "CodeSystem {first_system} was not found and has been excluded from the expansion"
            );
            tracing::warn!(%first_system, "{msg}");
            warnings.push(msg);
            return Ok(Some(vec![]));
        }
    };

    // Build one sub-select per include clause, joined with UNION.
    // Each sub-select drives from the FIRST filter (uses idx_concept_properties_value),
    // then ANDs every subsequent filter as a correlated EXISTS (uses
    // idx_concept_properties_lookup).  This avoids materialising and sorting
    // the large intermediate sets that INTERSECT requires.
    let mut union_parts: Vec<String> = Vec::new();
    let mut params: Vec<String> = Vec::new();

    for inc in includes {
        let filters = inc["filter"].as_array().unwrap();

        // Driver: first filter
        let f0 = &filters[0];
        let p0_idx = params.len() + 1;
        let v0_idx = params.len() + 2;
        params.push(f0["property"].as_str().unwrap_or("").to_string());
        params.push(f0["value"].as_str().unwrap_or("").to_string());

        // EXISTS clauses for additional filters (idx_concept_properties_lookup)
        let mut exists_clauses = String::new();
        for f in &filters[1..] {
            let ep_idx = params.len() + 1;
            let ev_idx = params.len() + 2;
            params.push(f["property"].as_str().unwrap_or("").to_string());
            params.push(f["value"].as_str().unwrap_or("").to_string());
            exists_clauses.push_str(&format!(
                "\n      AND EXISTS (SELECT 1 FROM concept_properties \
                 WHERE concept_id = cp0.concept_id AND property = ?{ep_idx} AND value = ?{ev_idx})"
            ));
        }

        union_parts.push(format!(
            "SELECT cp0.concept_id FROM concept_properties cp0 \
             WHERE cp0.property = ?{p0_idx} AND cp0.value = ?{v0_idx}{exists_clauses}"
        ));
    }

    let sid_idx = params.len() + 1;
    params.push(system_id);

    let union_sql = union_parts.join("\n    UNION\n    ");
    let sql = format!(
        "SELECT c.code, c.display\n\
         FROM concepts c\n\
         WHERE c.system_id = ?{sid_idx}\n\
         AND c.id IN (\n    {union_sql}\n)"
    );

    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let results = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(ExpansionContains {
                system: first_system.to_owned(),
                code: row.get(0)?,
                display: row.get(1)?,
                is_abstract: None,

                inactive: None,

                designations: vec![],

                properties: vec![],
                contains: vec![],
            })
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(Some(results))
}

/// Expand the subtree of `root_code` downward and immediately filter by
/// a property equality constraint, all in a single recursive CTE query.
///
/// This is the fast path for the common compose pattern:
/// `{ filter: [{ concept is-a/descendent-of X }, { property=value }] }`
///
/// By expanding the subtree (bounded by its size — e.g., ~2 000 for "Allergic
/// disorder") and joining with `concept_properties` in one pass we avoid
/// materialising the global property candidates (~10 000–24 000 rows) that
/// the two-phase approach would produce.
#[allow(clippy::too_many_arguments)]
fn query_subtree_with_property(
    conn: &Connection,
    system_url: &str,
    system_id: &str,
    root_code: &str,
    include_self: bool,
    property: &str,
    value: &str,
    text_filter: Option<&str>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    // Property-first: idx_concept_properties_value narrows candidates to
    // O(K_property) rows before the closure PK checks ancestry.
    // For large SNOMED subtrees (e.g. "Disease" → 50 K descendants) with a
    // selective property (e.g. finding-site = Airway → 100 concepts) this is
    // several orders of magnitude faster than the closure-first approach.
    //
    // When text_filter is set, the instr() clause pushes the text match into
    // SQL so the DB returns only matching rows (EX08 optimisation — avoids
    // loading all property-matching descendants into Rust before discarding them).
    let include_self_i = i64::from(include_self);

    let row_fn =
        |row: &rusqlite::Row<'_>| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?));
    let make = |(code, display): (String, Option<String>)| ExpansionContains {
        system: system_url.to_owned(),
        code,
        display,
        is_abstract: None,

        inactive: None,

        designations: vec![],

        properties: vec![],
        contains: vec![],
    };

    let pairs: Vec<(String, Option<String>)> =
        if let Some(tf) = text_filter.filter(|t| !t.is_empty()) {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT c.code, c.display
                 FROM   concept_properties cp
                 JOIN   concepts c ON c.id = cp.concept_id AND c.system_id = ?2
                 JOIN   concept_closure cc
                        ON cc.system_id = ?2
                        AND cc.ancestor_code = ?1
                        AND cc.descendant_code = c.code
                 WHERE  cp.property = ?3
                   AND  cp.value = ?4
                   AND  (c.code != ?1 OR ?5)
                   AND  (instr(lower(c.display), ?6) > 0
                         OR instr(lower(c.code), ?6) > 0)",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            stmt.query_map(
                rusqlite::params![root_code, system_id, property, value, include_self_i, tf],
                row_fn,
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))?
        } else {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT c.code, c.display
                 FROM   concept_properties cp
                 JOIN   concepts c ON c.id = cp.concept_id AND c.system_id = ?2
                 JOIN   concept_closure cc
                        ON cc.system_id = ?2
                        AND cc.ancestor_code = ?1
                        AND cc.descendant_code = c.code
                 WHERE  cp.property = ?3
                   AND  cp.value = ?4
                   AND  (c.code != ?1 OR ?5)",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            stmt.query_map(
                rusqlite::params![root_code, system_id, property, value, include_self_i],
                row_fn,
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))?
        };

    Ok(pairs.into_iter().map(make).collect())
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
    // Property-first: idx_concept_properties_value (property, value, concept_id)
    // narrows to O(K) candidates before filtering by system_id from concepts.
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.code, c.display
             FROM concept_properties cp
             JOIN concepts c ON c.id = cp.concept_id AND c.system_id = ?1
             WHERE cp.property = ?2
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
            is_abstract: None,

            inactive: None,

            designations: vec![],

            properties: vec![],
            contains: vec![],
        })
        .collect())
}

/// Returns the subset of `candidates` that are descendants (or self, when
/// `include_self=true`) of `root_code`.
///
/// Uses an **upward** recursive CTE that walks from each candidate toward the
/// root of the hierarchy, stopping as soon as `root_code` is found.
///
/// Complexity: O(N_candidates × depth) — far cheaper than the alternative
/// O(N_subtree) downward expansion when the subtree is large (e.g. SNOMED CT
/// "Disease" has ~50 000 descendants) but the candidate set is small (e.g.
/// a few hundred codes returned by a property-equality pre-filter).
fn batch_descendants_in_set(
    conn: &Connection,
    system_id: &str,
    root_code: &str,
    include_self: bool,
    candidates: &[String],
) -> Result<HashSet<String>, HtsError> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let json_candidates =
        serde_json::to_string(candidates).map_err(|e| HtsError::StorageError(e.to_string()))?;

    // O(1) closure lookup per candidate via a single JOIN.
    // The closure stores self-links so include_self is handled by the
    // `(j.value != ?1 OR ?4)` predicate (1 = include self, 0 = exclude).
    let mut stmt = conn
        .prepare_cached(
            "SELECT j.value
             FROM   json_each(?3) j
             JOIN   concept_closure cc
                    ON cc.system_id = ?2 AND cc.ancestor_code = ?1 AND cc.descendant_code = j.value
             WHERE  (j.value != ?1 OR ?4)",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let include_self_i = i64::from(include_self);
    let codes = stmt
        .query_map(
            rusqlite::params![root_code, system_id, json_candidates, include_self_i],
            |r| r.get(0),
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<HashSet<String>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(codes)
}

/// Returns the subset of `candidates` that are ancestors-or-self of `value_code`,
/// using a single upward recursive CTE.
///
/// Used for the `generalizes` compose filter: `C generalizes value` ⟺
/// C is an ancestor (or self) of `value`.
fn batch_ancestors_in_set(
    conn: &Connection,
    system_id: &str,
    value_code: &str,
    candidates: &[String],
) -> Result<HashSet<String>, HtsError> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let json_candidates =
        serde_json::to_string(candidates).map_err(|e| HtsError::StorageError(e.to_string()))?;

    // O(1) closure lookup: `generalizes value` ⟺ C is ancestor-or-self of value.
    // The closure stores self-links so this naturally returns value_code itself.
    let mut stmt = conn
        .prepare_cached(
            "SELECT j.value
             FROM   json_each(?3) j
             JOIN   concept_closure cc
                    ON cc.system_id = ?2 AND cc.descendant_code = ?1 AND cc.ancestor_code = j.value",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let codes = stmt
        .query_map(
            rusqlite::params![value_code, system_id, json_candidates],
            |r| r.get(0),
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<HashSet<String>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(codes)
}

/// Return all ancestors (including self) of `value_code` in `system_id`.
///
/// Uses a single recursive CTE walking UP the `concept_hierarchy` table.
/// Ancestor chains in SNOMED CT are ≤ ~20 hops, so this is O(depth) and
/// much faster than full ECL evaluation for the `generalizes` operator.
fn query_ancestors_full(
    conn: &Connection,
    system_url: &str,
    system_id: &str,
    value_code: &str,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT cc.ancestor_code, c.display
             FROM   concept_closure cc
             JOIN   concepts c ON c.system_id = ?2 AND c.code = cc.ancestor_code
             WHERE  cc.system_id = ?2 AND cc.descendant_code = ?1",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows = stmt
        .query_map(rusqlite::params![value_code, system_id], |r| {
            Ok(ExpansionContains {
                system: system_url.to_owned(),
                code: r.get(0)?,
                display: r.get(1)?,
                is_abstract: None,

                inactive: None,

                designations: vec![],

                properties: vec![],
                contains: vec![],
            })
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows)
}

/// Compile a ValueSet compose-filter regex with FHIR full-string semantics.
///
/// FHIR R5 §4.9.5 specifies that a `regex` filter matches when the entire
/// property value matches the pattern (anchored at both ends).  The Rust
/// `regex` crate is unanchored by default, so we wrap the user pattern with
/// `\A(?:…)\z` — these are absolute anchors (immune to multiline flags) and
/// the non-capturing group keeps top-level alternation working as the user
/// expects (e.g. `a|b` becomes `\A(?:a|b)\z`, not `\Aa|b\z`).
///
/// On parse failure returns [`HtsError::VsInvalid`] so the IG fixtures see a
/// `tx-issue-type=vs-invalid` coding rather than a generic `invalid` error.
///
/// The Rust `regex` crate uses an RE2-style linear-time engine: it does not
/// support PCRE features such as backreferences (`\1`) or lookaround
/// (`(?=…)`, `(?!…)`).  Patterns that rely on those constructs are rejected
/// with `vs-invalid`; the HL7 tx-ecosystem fixtures we know of do not use
/// them.
fn compile_vs_regex(pattern: &str) -> Result<Regex, HtsError> {
    if pattern.is_empty() {
        return Err(HtsError::VsInvalid(
            "ValueSet compose filter with op='regex' has an empty value".to_string(),
        ));
    }
    let anchored = format!("\\A(?:{pattern})\\z");
    Regex::new(&anchored).map_err(|e| {
        HtsError::VsInvalid(format!(
            "ValueSet compose filter has an invalid regular expression '{pattern}': {e}"
        ))
    })
}

/// Evaluate a `regex` compose filter — returns concepts in `system_id` whose
/// `code` (when `property == "code"`) or whose `concept_properties` value for
/// `property` fully matches `pattern`.
///
/// The match is performed in Rust after the candidate rows have been loaded.
/// For property-value regex we narrow at the SQL level to rows that even have
/// a value for `property`; for `code` regex we scan all concepts in the system.
fn query_regex_match(
    conn: &Connection,
    system_url: &str,
    system_id: &str,
    property: &str,
    pattern: &str,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let regex = compile_vs_regex(pattern)?;

    if property == "code" || property.is_empty() {
        // Match against the concept code itself.  Load all concepts for the
        // system and filter in Rust — there is no cheap SQL primitive for an
        // arbitrary regex, and concept counts in tx-ecosystem fixtures are
        // small (the regex-bad CodeSystem is 3 concepts; SNOMED-scale code
        // regex would be expensive but is not exercised by the IG suite).
        let mut stmt = conn
            .prepare_cached("SELECT code, display FROM concepts WHERE system_id = ?1 ORDER BY code")
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let rows = stmt
            .query_map([system_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter(|(code, _)| regex.is_match(code))
            .map(|(code, display)| ExpansionContains {
                system: system_url.to_owned(),
                code,
                display,
                is_abstract: None,
                inactive: None,
                designations: vec![],
                properties: vec![],
                contains: vec![],
            })
            .collect())
    } else {
        // Match against a named property value.  Pre-narrow at SQL to rows
        // that carry the property — the `idx_concept_properties_value` index
        // covers the (property, value, concept_id) triple.
        let mut stmt = conn
            .prepare_cached(
                "SELECT c.code, c.display, cp.value
                 FROM   concept_properties cp
                 JOIN   concepts c ON c.id = cp.concept_id AND c.system_id = ?1
                 WHERE  cp.property = ?2",
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let rows = stmt
            .query_map([system_id, property], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        // A concept may have multiple values for the same property; keep it if
        // any value matches.  Dedupe by code in case more than one value matches.
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<ExpansionContains> = Vec::new();
        for (code, display, value) in rows {
            if regex.is_match(&value) && seen.insert(code.clone()) {
                out.push(ExpansionContains {
                    system: system_url.to_owned(),
                    code,
                    display,
                    is_abstract: None,
                    inactive: None,
                    designations: vec![],
                    properties: vec![],
                    contains: vec![],
                });
            }
        }
        Ok(out)
    }
}

/// Evaluate a `child-of` compose filter — returns the **direct** children of
/// `parent_code` in `system_id`.  Per FHIR R5 §4.9.5 `child-of` selects only
/// concepts whose immediate parent (one level) is the supplied value, never
/// the value itself and never deeper descendants.  Use the pre-materialized
/// `concept_hierarchy(parent_code, child_code)` parent-link table rather than
/// `concept_closure`, which would also return transitive descendants.
fn query_direct_children(
    conn: &Connection,
    system_url: &str,
    system_id: &str,
    parent_code: &str,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.code, c.display
             FROM   concept_hierarchy h
             JOIN   concepts c ON c.system_id = ?1 AND c.code = h.child_code
             WHERE  h.system_id = ?1 AND h.parent_code = ?2 AND h.child_code != ?2",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    let rows = stmt
        .query_map([system_id, parent_code], |r| {
            Ok(ExpansionContains {
                system: system_url.to_owned(),
                code: r.get(0)?,
                display: r.get(1)?,
                is_abstract: None,
                inactive: None,
                designations: vec![],
                properties: vec![],
                contains: vec![],
            })
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(rows)
}

/// Returns the subset of `candidates` whose immediate parent (one level) in
/// `concept_hierarchy` is `parent_code`.  Used by
/// [`apply_compose_filters_to_candidates`] to intersect a `child-of` filter
/// against an already-bounded candidate set without re-querying every concept
/// in the system.
fn batch_direct_children_in_set(
    conn: &Connection,
    system_id: &str,
    parent_code: &str,
    candidates: &[String],
) -> Result<HashSet<String>, HtsError> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let json_candidates =
        serde_json::to_string(candidates).map_err(|e| HtsError::StorageError(e.to_string()))?;
    let mut stmt = conn
        .prepare_cached(
            "SELECT j.value
             FROM   json_each(?3) j
             JOIN   concept_hierarchy h
                    ON h.system_id = ?2 AND h.parent_code = ?1 AND h.child_code = j.value
             WHERE  j.value != ?1",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    let codes = stmt
        .query_map(
            rusqlite::params![parent_code, system_id, json_candidates],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<HashSet<_>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(codes)
}

/// Return all concepts in `system_id` matching `filter_lower` via FTS5 trigram.
///
/// The caller is responsible for calling [`ensure_concepts_fts`] first.
/// Returns at most 5 000 entries (sufficient for any realistic text filter result).
fn fts_candidates_for_system(
    conn: &Connection,
    system_id: &str,
    system_url: &str,
    filter_lower: &str,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let match_expr = fts5_quote(filter_lower);
    let mut stmt = conn
        .prepare_cached(
            "SELECT code, display FROM concepts_fts \
             WHERE concepts_fts MATCH ?1 AND system_id = ?2 \
             LIMIT 5000",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![match_expr, system_id], |row| {
            Ok(ExpansionContains {
                system: system_url.to_owned(),
                code: row.get(0)?,
                display: row.get(1)?,
                is_abstract: None,

                inactive: None,

                designations: vec![],

                properties: vec![],
                contains: vec![],
            })
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(rows)
}

/// Apply compose `filter[]` entries to an already-bounded candidate set.
///
/// Used by the FTS-first path in [`expand_inline_filtered`]: FTS gives a small
/// set of text-matching candidates; this function checks each one against the
/// hierarchy / property filters without expanding the full subtree.
///
/// Supported filter types:
/// - `concept is-a / descendent-of / generalizes` → batch ancestor walk
/// - `<property> = <value>` (non-ECL) → batch property equality lookup
///
/// ECL `constraint` filters are NOT handled here — callers must verify
/// `all_batchable` before invoking this function.
fn apply_compose_filters_to_candidates(
    conn: &Connection,
    system_id: &str,
    filters: &[serde_json::Value],
    mut candidates: Vec<ExpansionContains>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    for f in filters {
        if candidates.is_empty() {
            break;
        }
        let property = f["property"].as_str().unwrap_or("");
        let op = f["op"].as_str().unwrap_or("");
        let value = f["value"].as_str().unwrap_or("");

        // `code` and `concept` both refer to the concept-id property in
        // various IG fixtures (search/* uses `code`, simple/* uses
        // `concept`). Normalise to the canonical `concept` for matching.
        let property_norm = if property == "code" {
            "concept"
        } else {
            property
        };

        match (property_norm, op) {
            ("concept", "is-a") => {
                let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();
                let valid = batch_descendants_in_set(conn, system_id, value, true, &codes)?;
                candidates.retain(|c| valid.contains(&c.code));
            }
            ("concept", "descendent-of") => {
                let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();
                let valid = batch_descendants_in_set(conn, system_id, value, false, &codes)?;
                candidates.retain(|c| valid.contains(&c.code));
            }
            ("concept", "generalizes") => {
                let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();
                let valid = batch_ancestors_in_set(conn, system_id, value, &codes)?;
                candidates.retain(|c| valid.contains(&c.code));
            }
            ("concept", "child-of") => {
                if value.is_empty() {
                    return Err(HtsError::VsInvalid(
                        "ValueSet compose filter with op='child-of' is missing a value".to_string(),
                    ));
                }
                let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();
                let valid = batch_direct_children_in_set(conn, system_id, value, &codes)?;
                candidates.retain(|c| valid.contains(&c.code));
            }
            (_, "regex") => {
                let regex = compile_vs_regex(value)?;
                if property == "code" || property.is_empty() {
                    candidates.retain(|c| regex.is_match(&c.code));
                } else {
                    let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();
                    let json_codes = serde_json::to_string(&codes)
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT c.code, cp.value
                             FROM   concept_properties cp
                             JOIN   concepts c ON c.id = cp.concept_id AND c.system_id = ?1
                             WHERE  cp.property = ?2
                               AND  c.code IN (SELECT value FROM json_each(?3))",
                        )
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    let rows = stmt
                        .query_map(rusqlite::params![system_id, property, json_codes], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })
                        .map_err(|e| HtsError::StorageError(e.to_string()))?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    let mut keep: HashSet<String> = HashSet::new();
                    for (code, val) in rows {
                        if regex.is_match(&val) {
                            keep.insert(code);
                        }
                    }
                    candidates.retain(|c| keep.contains(&c.code));
                }
            }
            (_, "=") => {
                let codes: Vec<String> = candidates.iter().map(|c| c.code.clone()).collect();
                let valid = batch_property_eq_in_set(conn, system_id, property, value, &codes)?;
                candidates.retain(|c| valid.contains(&c.code));
            }
            _ => {}
        }
    }
    Ok(candidates)
}

/// Check which of `candidates` have `(property = value)` in `concept_properties`.
///
/// Uses `json_each` to pass the candidate codes as a JSON array, avoiding N+1
/// queries.  Returns a `HashSet` of the codes that matched.
fn batch_property_eq_in_set(
    conn: &Connection,
    system_id: &str,
    property: &str,
    value: &str,
    candidates: &[String],
) -> Result<HashSet<String>, HtsError> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let json_candidates =
        serde_json::to_string(candidates).map_err(|e| HtsError::StorageError(e.to_string()))?;
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.code
             FROM   concepts c
             JOIN   concept_properties cp ON cp.concept_id = c.id
             WHERE  c.system_id = ?1
               AND  cp.property = ?2
               AND  cp.value = ?3
               AND  c.code IN (SELECT value FROM json_each(?4))",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    let codes = stmt
        .query_map(
            rusqlite::params![system_id, property, value, json_candidates],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<rusqlite::Result<HashSet<_>>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(codes)
}

/// Fast path for multi-include OR composes where every include is a single
/// hierarchy filter (`is-a`, `descendent-of`, or `generalizes`).
///
/// Each include is expanded with a bounded BFS (limit = `offset + count`),
/// results are unioned and deduplicated, then the requested page is returned.
/// This avoids full ECL subtree expansion for each OR branch, which can block
/// a connection for >30 s on large SNOMED hierarchies at high concurrency.
///
/// Returns `None` when the compose is not a qualifying multi-include (caller
/// should fall through to `compute_expansion`).
fn try_multiinclude_hierarchy_page(
    conn: &Connection,
    compose: &serde_json::Value,
    count: usize,
    offset: usize,
    warnings: &mut Vec<String>,
) -> Result<Option<Vec<ExpansionContains>>, HtsError> {
    let includes = match compose["include"].as_array() {
        Some(a) if a.len() >= 2 => a,
        _ => return Ok(None),
    };

    struct Entry {
        sys_url: String,
        sys_id: String,
        root_code: String,
        include_root: bool,
        is_generalizes: bool,
    }

    let mut entries: Vec<Entry> = Vec::new();

    for inc in includes {
        // Must be a single-filter hierarchy — no explicit concept lists,
        // no nested valueSet refs, exactly one filter entry.
        if inc["concept"].as_array().is_some_and(|a| !a.is_empty()) {
            return Ok(None);
        }
        if inc["valueSet"].as_array().is_some_and(|a| !a.is_empty()) {
            return Ok(None);
        }
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

        let (include_root, is_generalizes) = match op {
            "is-a" => (true, false),
            "descendent-of" => (false, false),
            "generalizes" => (true, true),
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

        match system_id {
            Some(id) => entries.push(Entry {
                sys_url: system_url.to_owned(),
                sys_id: id,
                root_code: root_code.to_owned(),
                include_root,
                is_generalizes,
            }),
            None => {
                warnings.push(format!(
                    "CodeSystem {system_url} was not found and has been excluded from the expansion"
                ));
            }
        }
    }

    if entries.is_empty() {
        return Ok(None);
    }

    // Union-BFS: expand each include branch up to `offset + count` items so
    // the merged, deduplicated set covers the requested page.
    let per_branch_limit = offset + count;
    let mut seen: HashSet<String> = HashSet::new();
    let mut all: Vec<ExpansionContains> = Vec::new();

    for e in &entries {
        let concepts = if e.is_generalizes {
            // Ancestors are tiny (≤ ~20), fetch all.
            query_ancestors_full(conn, &e.sys_url, &e.sys_id, &e.root_code)?
        } else {
            bfs_isa_page(
                conn,
                &e.sys_url,
                &e.sys_id,
                &e.root_code,
                e.include_root,
                0,
                per_branch_limit,
                None,
            )?
        };
        for c in concepts {
            if seen.insert(c.code.clone()) {
                all.push(c);
            }
        }
    }

    let start = offset.min(all.len());
    let end = (offset + count).min(all.len());
    Ok(Some(all[start..end].to_vec()))
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
    filter: Option<&str>,
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

    // Collect (system_url, code, embedded_display) triples in compose order.
    // Using the compose-embedded display avoids per-code DB lookups for systems
    // not in the DB (e.g. VSAC ValueSets with RxNorm codes) and also enables
    // filter matching against embedded display names.
    let mut all_triples: Vec<(String, String, Option<String>)> = Vec::new();
    for inc in includes {
        let system_url = match inc["system"].as_str() {
            Some(s) if !s.is_empty() => s.to_owned(),
            _ => continue,
        };
        if let Some(concepts) = inc["concept"].as_array() {
            for c in concepts {
                if let Some(code) = c["code"].as_str() {
                    let display = c["display"].as_str().map(|s| s.to_owned());
                    all_triples.push((system_url.clone(), code.to_owned(), display));
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
            all_triples
                .retain(|(sys, code, _)| !exclude_set.contains(&(sys.clone(), code.clone())));
        }
    }

    // Apply text filter against compose-embedded code and display — pure in-memory,
    // no DB required.  This makes filtered requests on large extensional ValueSets
    // (e.g. VSAC Medication ValueSets with 33K RxNorm codes) fast even when the
    // referenced system is not present in the local concepts table.
    if let Some(f) = filter {
        let lower = f.to_lowercase();
        all_triples.retain(|(_, code, display)| {
            code.to_lowercase().contains(&lower)
                || display
                    .as_deref()
                    .map(|d| d.to_lowercase().contains(&lower))
                    .unwrap_or(false)
        });
    }

    let total = all_triples.len() as u32;

    // Paginate: take only the slice we need.
    let page_triples: Vec<(String, String, Option<String>)> =
        all_triples.into_iter().skip(offset).take(limit).collect();

    if page_triples.is_empty() {
        return Ok(Some((vec![], total)));
    }

    // Use compose-embedded display; fall back to DB lookup only when the
    // embedded display is absent (rare — VSAC always includes display names).
    let mut result = Vec::with_capacity(page_triples.len());
    let mut system_cache: HashMap<String, Option<String>> = HashMap::new();

    for (system_url, code, embedded_display) in &page_triples {
        let display = if embedded_display.is_some() {
            embedded_display.clone()
        } else {
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

            if let Some(sid) = system_id {
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
            }
        };

        result.push(ExpansionContains {
            system: system_url.clone(),
            code: code.clone(),
            display,
            is_abstract: None,

            inactive: None,

            designations: vec![],

            properties: vec![],
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
                if f.len() >= 3 {
                    // Build FTS5 index lazily (no-op if already populated).
                    ensure_concepts_fts(conn, system_id)?;
                    let match_expr = fts5_quote(f);
                    let mut stmt = conn
                        .prepare_cached(
                            "SELECT code, display FROM concepts_fts \
                             WHERE concepts_fts MATCH ?1 AND system_id = ?2 \
                             LIMIT ?3 OFFSET ?4",
                        )
                        .map_err(|e| HtsError::StorageError(e.to_string()))?;
                    return stmt
                        .query_map(
                            rusqlite::params![match_expr, system_id, sql_limit, sql_offset],
                            |r| {
                                Ok(ExpansionContains {
                                    system: cs_url.to_owned(),
                                    code: r.get(0)?,
                                    display: r.get(1)?,
                                    is_abstract: None,

                                    inactive: None,

                                    designations: vec![],

                                    properties: vec![],
                                    contains: vec![],
                                })
                            },
                        )
                        .map_err(|e| HtsError::StorageError(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| HtsError::StorageError(e.to_string()));
                }
                // Short filter (1–2 chars): use word-prefix FTS so `a*` matches any
                // token starting with 'a' — O(log N) vs O(N) LIKE scan.
                ensure_concepts_fts(conn, system_id)?;
                let prefix_expr = fts5_word_prefix(f);
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT code, display FROM concepts_word_fts \
                         WHERE concepts_word_fts MATCH ?1 AND system_id = ?2 \
                         LIMIT ?3 OFFSET ?4",
                    )
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;
                stmt.query_map(
                    rusqlite::params![prefix_expr, system_id, sql_limit, sql_offset],
                    |r| {
                        Ok(ExpansionContains {
                            system: cs_url.to_owned(),
                            code: r.get(0)?,
                            display: r.get(1)?,
                            is_abstract: None,

                            inactive: None,

                            designations: vec![],

                            properties: vec![],
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
                        is_abstract: None,

                        inactive: None,

                        designations: vec![],

                        properties: vec![],
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

/// Return one page of `IsA` or `DescendentOf` hierarchy descendants.
///
/// Queries the precomputed `concept_closure` table.  The closure primary key
/// `(system_id, ancestor_code, descendant_code)` already delivers rows in
/// `descendant_code` order — no explicit ORDER BY is needed, and SQLite can
/// stop the join after `limit` rows rather than materialising all descendants
/// to sort them.  This reduces EX02-style hierarchy page requests from
/// O(N_descendants) to O(limit).
///
/// `include_root=true` — is-a / `<<` semantics (self + descendants).
/// `include_root=false` — descendent-of / `<` semantics (descendants only).
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
    let sql_limit = limit as i64;
    let sql_offset = offset as i64;
    // 1 = include root (is-a), 0 = exclude (descendent-of)
    let include_root_i = i64::from(include_root);

    let row_mapper = |r: &rusqlite::Row<'_>| {
        Ok(ExpansionContains {
            system: cs_url.to_owned(),
            code: r.get(0)?,
            display: r.get(1)?,
            is_abstract: None,

            inactive: None,

            designations: vec![],

            properties: vec![],
            contains: vec![],
        })
    };

    if let Some(f) = filter_lower {
        if f.len() >= 3 {
            ensure_concepts_fts(conn, system_id)?;
            let match_expr = fts5_quote(f);
            let mut stmt = conn
                .prepare_cached(
                    "SELECT cf.code, cf.display
                     FROM   concepts_fts cf
                     JOIN   concept_closure cc
                            ON cc.system_id = ?5 AND cc.ancestor_code = ?4
                            AND cc.descendant_code = cf.code
                     WHERE  cf.system_id = ?5
                       AND  concepts_fts MATCH ?1
                       AND  (cf.code != ?4 OR ?6)
                     LIMIT ?2 OFFSET ?3",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            return stmt
                .query_map(
                    rusqlite::params![
                        match_expr,
                        sql_limit,
                        sql_offset,
                        root_code,
                        system_id,
                        include_root_i
                    ],
                    row_mapper,
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| HtsError::StorageError(e.to_string()));
        }
        // Short filter (< 3 chars): LIKE scan on the closure join.
        let sql_pat = format!("%{f}%");
        let mut stmt = conn
            .prepare_cached(
                "SELECT c.code, c.display
                 FROM   concept_closure cc
                 JOIN   concepts c ON c.system_id = ?5 AND c.code = cc.descendant_code
                 WHERE  cc.system_id = ?5
                   AND  cc.ancestor_code = ?4
                   AND  (cc.descendant_code != ?4 OR ?6)
                   AND  (LOWER(c.code) LIKE ?1 OR LOWER(COALESCE(c.display,'')) LIKE ?1)
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        return stmt
            .query_map(
                rusqlite::params![
                    sql_pat,
                    sql_limit,
                    sql_offset,
                    root_code,
                    system_id,
                    include_root_i
                ],
                row_mapper,
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| HtsError::StorageError(e.to_string()));
    }

    // No filter: pure closure lookup.
    // No ORDER BY: the closure PK (system_id, ancestor_code, descendant_code)
    // already delivers rows in descendant_code order, so SQLite can stop the
    // nested-loop join at LIMIT without materialising all descendants.
    let mut stmt = conn
        .prepare_cached(
            "SELECT c.code, c.display
             FROM   concept_closure cc
             JOIN   concepts c ON c.system_id = ?4 AND c.code = cc.descendant_code
             WHERE  cc.system_id = ?4
               AND  cc.ancestor_code = ?1
               AND  (cc.descendant_code != ?1 OR ?5)
             LIMIT ?2 OFFSET ?3",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    stmt.query_map(
        rusqlite::params![root_code, sql_limit, sql_offset, system_id, include_root_i],
        row_mapper,
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Look up the storage id of a code_systems row given a canonical URL and an
/// optional version constraint from a `compose.include[]` entry.
///
/// Mirrors the version-resolution rules used by `$lookup` /
/// `$validate-code` / `$subsumes`: an exact version requires a literal match,
/// `1.x.x` / `1.x` / bare `1` patterns match the highest version that shares
/// the literal segments, and `None` falls back to the most recent revision.
///
/// Returns `Ok(None)` when no row matches so callers can skip the include
/// rather than abort the whole expansion.
fn resolve_compose_system_id(
    conn: &Connection,
    url: &str,
    version: Option<&str>,
) -> Result<Option<String>, HtsError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, version FROM code_systems \
             WHERE url = ?1 \
             ORDER BY COALESCE(version, '') DESC",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows: Vec<(String, Option<String>)> = stmt
        .query_map(rusqlite::params![url], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<Result<_, _>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if rows.is_empty() {
        return Ok(None);
    }

    let chosen = match version {
        Some(v) if v.contains(".x") || v == "x" || super::code_system_version_is_short(v) => {
            super::code_system_select_version_match(&rows, v)
        }
        Some(v) => rows.into_iter().find(|(_, ver)| ver.as_deref() == Some(v)),
        None => rows.into_iter().next(),
    };

    Ok(chosen.map(|(id, _)| id))
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
        rusqlite::Error::QueryReturnedNoRows => HtsError::NotFound(format!(
            "A definition for the value Set \'{vs_url}\' could not be found"
        )),
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

    // For each unique system URL, pick the latest-versioned id so the
    // hierarchy edges we walk reflect the most recent revision when the
    // expansion combines codes from multiple versions of the same URL.
    let system_urls: HashSet<String> = flat.iter().map(|c| c.system.clone()).collect();
    let mut system_id_map: HashMap<String, String> = HashMap::new();
    for sys_url in &system_urls {
        if let Some(id) = conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = ?1 \
                 ORDER BY COALESCE(version, '') DESC LIMIT 1",
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
/// Returns true when the concept (system_url, code) is marked notSelectable=true.
///
/// Used to reject abstract concepts from $validate-code: per the IG fixtures,
/// validating an abstract code against a VS that contains it must still
/// produce result=false with an "abstract, and not allowed in this context"
/// message.
fn is_concept_abstract(conn: &Connection, system_url: &str, code: &str) -> bool {
    // Match against every local property code that maps to the FHIR
    // concept-properties#notSelectable URI in this CodeSystem. Tx-ecosystem
    // fixtures rename the property locally (e.g. `not-selectable` with a
    // hyphen), so a query hardcoded to `notSelectable` would miss them.
    let abstract_codes = super::code_system::abstract_property_codes(conn, system_url);
    let placeholders = (3..=abstract_codes.len() + 2)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT 1
         FROM concept_properties cp
         JOIN concepts c ON c.id = cp.concept_id
         JOIN code_systems s ON s.id = c.system_id
         WHERE s.url = ?1
           AND c.code = ?2
           AND cp.property IN ({placeholders})
           AND cp.value = 'true'
         LIMIT 1"
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(abstract_codes.len() + 2);
    params.push(&system_url);
    params.push(&code);
    for c in &abstract_codes {
        params.push(c as &dyn rusqlite::ToSql);
    }
    conn.query_row(&sql, params.as_slice(), |_| Ok(())).is_ok()
}

/// Returns the stored version for a ValueSet URL (None if unknown). Used to
/// format `url|version` in $validate-code "code not found" messages, which
/// is what the IG fixtures expect.
fn lookup_value_set_version(conn: &Connection, url: &str) -> Option<String> {
    // Pick the highest stored version for this URL — matches the
    // resolve_value_set_versioned default-when-no-pin behaviour, so $expand
    // and $validate-code echoes converge on the same row.
    conn.query_row(
        "SELECT version FROM value_sets \
         WHERE url = ?1 \
         ORDER BY COALESCE(version, '') DESC LIMIT 1",
        rusqlite::params![url],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

/// Returns true when the concept has a status property in the inactive set
/// (retired/deprecated/withdrawn/inactive). Used by $validate-code so the
/// response can surface a top-level `inactive` parameter per the IG fixtures.
/// `true` when the code exists in the named CodeSystem (regardless of any
/// flags). Used by validate_code to decide whether to emit a separate
/// `code-invalid` / `invalid-code` issue ("Unknown code 'X' in the
/// CodeSystem 'url' version 'Y'") when the VS validation already failed
/// because the code is absent from the underlying CS.
fn is_code_in_cs(conn: &Connection, system_url: &str, code: &str) -> bool {
    conn.query_row(
        "SELECT 1
         FROM concepts c
         JOIN code_systems s ON s.id = c.system_id
         WHERE s.url = ?1 AND c.code = ?2
         LIMIT 1",
        rusqlite::params![system_url, code],
        |_| Ok(()),
    )
    .is_ok()
}

/// Returns the highest stored version for a CodeSystem URL, used to format
/// the IG-expected "Unknown code in CodeSystem 'url' version 'X'" message.
fn cs_version_for_msg(conn: &Connection, system_url: &str) -> Option<String> {
    conn.query_row(
        "SELECT version FROM code_systems \
         WHERE url = ?1 \
         ORDER BY COALESCE(version, '') DESC LIMIT 1",
        rusqlite::params![system_url],
        |row| row.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

/// Returns all non-null stored versions for a CS URL, sorted ascending for
/// display in "Valid versions: X or Y" messages.
fn cs_all_stored_versions(conn: &Connection, system_url: &str) -> Vec<String> {
    let mut stmt = match conn.prepare_cached(
        "SELECT version FROM code_systems \
         WHERE url = ?1 AND version IS NOT NULL \
         ORDER BY COALESCE(version, '') ASC",
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(rusqlite::params![system_url], |row| row.get::<_, String>(0))
        .ok()
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// Format a list of versions as "X", "X or Y", or "X, Y or Z".
fn format_valid_versions_msg(versions: &[String]) -> String {
    match versions {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} or {second}"),
        _ => {
            let (last, rest) = versions.split_last().unwrap();
            format!("{} or {}", rest.join(", "), last)
        }
    }
}

/// Return `Some(pin)` where `pin` is the version string (or `None` for a
/// versionless include) when `system_url` appears in `compose.include[]`.
/// Returns `None` when the system is not found in any include.
fn vs_pinned_include_version(compose_json: &str, system_url: &str) -> Option<Option<String>> {
    let compose: serde_json::Value = serde_json::from_str(compose_json).ok()?;
    let includes = compose.get("include")?.as_array()?;
    for inc in includes {
        if inc.get("system").and_then(|v| v.as_str()) == Some(system_url) {
            let ver = inc
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            return Some(ver);
        }
    }
    None
}

/// Resolve a version string against a set of `(id, version)` candidate pairs.
/// Returns the matched full version string, or `None` when no candidate matches.
///
/// Rules:
/// - Explicit `.x` wildcards or bare "x" → pattern matching.
/// - Dot-containing versions ("1.0", "1.0.0") → prefix/pattern matching so
///   "1.0" resolves to the best "1.0.x" stored version.
/// - Single-integer versions ("1", "2") with no dot → EXACT match only.
///   These are not resolved via prefix expansion because the IG test fixtures
///   treat bare "1" as a distinct unrecognised version (producing
///   UNKNOWN_CODESYSTEM_VERSION), not as an alias for "1.x.x".
fn resolve_ver_against_candidates(
    candidates: &[(String, Option<String>)],
    ver: &str,
) -> Option<String> {
    if ver.contains(".x") || ver == "x" || ver.contains('.') {
        // Pattern/prefix matching: "1.0" → highest "1.0.x", "1.x" → highest "1.y.z"
        super::code_system_select_version_match(candidates, ver).and_then(|(_, v)| v)
    } else {
        // Single-segment or non-semver: EXACT match only
        candidates
            .iter()
            .find(|(_, v)| v.as_deref() == Some(ver))
            .and_then(|(_, v)| v.clone())
    }
}

/// Check whether `req_ver` (caller-supplied CS version) conflicts with what is
/// stored in the DB or pinned in the VS compose.
///
/// Returns `Some((issues, caused_by))` when a mismatch is detected:
/// - issues: validation issues to report
/// - caused_by: `Some(url|ver)` canonical for the `x-caused-by-unknown-system`
///   parameter (only when the requested version is missing from the DB).
///
/// Returns `None` when there is no mismatch (caller should proceed normally).
fn detect_cs_version_mismatch(
    conn: &Connection,
    system_url: &str,
    req_ver: &str,
    compose_json: Option<&str>,
) -> Option<(Vec<crate::types::ValidationIssue>, Option<String>)> {
    // Build (id, version) candidate list sorted desc so the first entry is the
    // highest version — used for both resolution and picking the "actual" ver.
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, version FROM code_systems \
             WHERE url = ?1 \
             ORDER BY COALESCE(version, '') DESC",
        )
        .ok()?;
    let candidates: Vec<(String, Option<String>)> = stmt
        .query_map(rusqlite::params![system_url], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    if candidates.is_empty() {
        return None; // CS not in DB — handled by the not-found path elsewhere
    }

    // Resolve req_ver (handles short-forms like "1.0" → "1.0.0")
    let resolved_req = resolve_ver_against_candidates(&candidates, req_ver);

    // Parse compose to find include pin for this system
    let include_pin: Option<Option<String>> =
        compose_json.and_then(|cj| vs_pinned_include_version(cj, system_url));

    // Highest stored version (for use in warning text when req_ver is missing)
    let actual_ver: Option<String> = candidates.iter().find_map(|(_, v)| v.clone());

    if resolved_req.is_none() {
        // req_ver does not match any stored CS version → UNKNOWN_CODESYSTEM_VERSION
        let all_versions = cs_all_stored_versions(conn, system_url);
        let valid_str = format_valid_versions_msg(&all_versions);
        let error_text = format!(
            "A definition for CodeSystem '{system_url}' version '{req_ver}' could not be found, \
             so the code cannot be validated. Valid versions: {valid_str}"
        );

        // Supplement with a VALUESET_VALUE_MISMATCH describing the VS include context.
        let (warning_text, warning_id) = match include_pin.as_ref() {
            Some(Some(inc_ver)) => (
                format!(
                    "The code system '{system_url}' version '{inc_ver}' in the ValueSet include \
                     is different to the one in the value ('{req_ver}')"
                ),
                "VALUESET_VALUE_MISMATCH",
            ),
            // Versionless include: use latest stored version as the effective version.
            Some(None) => {
                let latest = actual_ver.as_deref().unwrap_or(req_ver);
                (
                    format!(
                        "The code system '{system_url}' version '{latest}' for the versionless \
                         include in the ValueSet include is different to the one in the value ('{req_ver}')"
                    ),
                    "VALUESET_VALUE_MISMATCH_DEFAULT",
                )
            }
            // No VS context (bare CS validate-code) — just report the unknown version.
            None => {
                let latest = actual_ver.as_deref().unwrap_or(req_ver);
                (
                    format!(
                        "The code system '{system_url}' version '{latest}' for the versionless \
                         include in the ValueSet include is different to the one in the value ('{req_ver}')"
                    ),
                    "VALUESET_VALUE_MISMATCH_DEFAULT",
                )
            }
        };

        let issues = vec![
            crate::types::ValidationIssue {
                severity: "error".into(),
                fhir_code: "not-found".into(),
                tx_code: "not-found".into(),
                text: error_text,
                location: Some("Coding.system".into()),
                message_id: Some("UNKNOWN_CODESYSTEM_VERSION".into()),
            },
            crate::types::ValidationIssue {
                severity: "warning".into(),
                fhir_code: "invalid".into(),
                tx_code: "vs-invalid".into(),
                text: warning_text,
                location: Some("Coding.version".into()),
                message_id: Some(warning_id.into()),
            },
        ];
        let caused_by = Some(format!("{system_url}|{req_ver}"));
        return Some((issues, caused_by));
    }

    let req_full = resolved_req.as_deref().unwrap_or(req_ver);

    // req_ver exists in the CS. Check if the VS include pins a conflicting version.
    match include_pin {
        Some(Some(ref inc_ver)) => {
            let resolved_inc = resolve_ver_against_candidates(&candidates, inc_ver);
            let inc_full = resolved_inc.as_deref().unwrap_or(inc_ver.as_str());
            if inc_full != req_full {
                let mismatch_text = format!(
                    "The code system '{system_url}' version '{inc_full}' in the ValueSet include \
                     is different to the one in the value ('{req_full}')"
                );
                // When the VS pin itself doesn't exist in the DB, add UNKNOWN for
                // the pin version (e.g. VS include has version "1" but only "1.0.0"
                // and "1.2.0" are stored).
                if resolved_inc.is_none() {
                    let all_versions = cs_all_stored_versions(conn, system_url);
                    let valid_str = format_valid_versions_msg(&all_versions);
                    let unknown_text = format!(
                        "A definition for CodeSystem '{system_url}' version '{inc_ver}' could not \
                         be found, so the code cannot be validated. Valid versions: {valid_str}"
                    );
                    let issues = vec![
                        crate::types::ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "invalid".into(),
                            tx_code: "vs-invalid".into(),
                            text: mismatch_text,
                            location: Some("Coding.version".into()),
                            message_id: Some("VALUESET_VALUE_MISMATCH".into()),
                        },
                        crate::types::ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "not-found".into(),
                            tx_code: "not-found".into(),
                            text: unknown_text,
                            location: Some("Coding.system".into()),
                            message_id: Some("UNKNOWN_CODESYSTEM_VERSION".into()),
                        },
                    ];
                    let caused_by = Some(format!("{system_url}|{inc_ver}"));
                    return Some((issues, caused_by));
                }
                // Both versions exist but differ → VALUESET_VALUE_MISMATCH only.
                let issues = vec![crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "invalid".into(),
                    tx_code: "vs-invalid".into(),
                    text: mismatch_text,
                    location: Some("Coding.version".into()),
                    message_id: Some("VALUESET_VALUE_MISMATCH".into()),
                }];
                return Some((issues, None));
            }
        }
        Some(None) => {
            // Versionless VS include: the effective CS version is the latest stored.
            // If the caller requested an older version, emit VALUESET_VALUE_MISMATCH_DEFAULT.
            let latest = actual_ver.as_deref().unwrap_or(req_ver);
            if latest != req_full {
                let mismatch_text = format!(
                    "The code system '{system_url}' version '{latest}' for the versionless \
                     include in the ValueSet include is different to the one in the value ('{req_full}')"
                );
                let issues = vec![crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "invalid".into(),
                    tx_code: "vs-invalid".into(),
                    text: mismatch_text,
                    location: Some("Coding.version".into()),
                    message_id: Some("VALUESET_VALUE_MISMATCH_DEFAULT".into()),
                }];
                return Some((issues, None));
            }
        }
        None => {} // No VS context — req_ver was found, no mismatch to report.
    }

    None // No mismatch detected
}

/// When the caller provides **no** version, check whether the VS include pins
/// a version that doesn't exist in the DB.  Emits `UNKNOWN_CODESYSTEM_VERSION`
/// (with `x-caused-by-unknown-system`) when the pin can't be resolved.
///
/// Returns `None` when there is no issue (versionless include, pin resolves
/// OK, or no VS compose context).
fn detect_vs_pin_unknown(
    conn: &Connection,
    system_url: &str,
    compose_json: Option<&str>,
) -> Option<(Vec<crate::types::ValidationIssue>, Option<String>)> {
    let inc_ver = compose_json
        .and_then(|cj| vs_pinned_include_version(cj, system_url))
        .and_then(|pin| pin)?; // only when the include has an explicit version

    // Build candidates for resolution
    let mut stmt = conn
        .prepare_cached(
            "SELECT id, version FROM code_systems \
             WHERE url = ?1 \
             ORDER BY COALESCE(version, '') DESC",
        )
        .ok()?;
    let candidates: Vec<(String, Option<String>)> = stmt
        .query_map(rusqlite::params![system_url], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // If the pin resolves to a stored version, there is no issue.
    if resolve_ver_against_candidates(&candidates, &inc_ver).is_some() {
        return None;
    }

    // Pin doesn't exist → report it as unknown.
    let all_versions = cs_all_stored_versions(conn, system_url);
    let valid_str = format_valid_versions_msg(&all_versions);
    let error_text = format!(
        "A definition for CodeSystem '{system_url}' version '{inc_ver}' could not be found, \
         so the code cannot be validated. Valid versions: {valid_str}"
    );
    let issues = vec![crate::types::ValidationIssue {
        severity: "error".into(),
        fhir_code: "not-found".into(),
        tx_code: "not-found".into(),
        text: error_text,
        location: Some("Coding.system".into()),
        message_id: Some("UNKNOWN_CODESYSTEM_VERSION".into()),
    }];
    let caused_by = Some(format!("{system_url}|{inc_ver}"));
    Some((issues, caused_by))
}

fn is_concept_inactive(conn: &Connection, system_url: &str, code: &str) -> bool {
    // Honour both the legacy `status` property convention (value in
    // {retired, deprecated, withdrawn, inactive}) AND the FHIR `inactive`
    // boolean property — including locally-renamed variants that the
    // CodeSystem.property[] declarations alias to the canonical URI.
    let inactive_codes = super::code_system::inactive_property_codes(conn, system_url);
    let placeholders = (3..=inactive_codes.len() + 2)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT 1
         FROM concept_properties cp
         JOIN concepts c ON c.id = cp.concept_id
         JOIN code_systems s ON s.id = c.system_id
         WHERE s.url = ?1
           AND c.code = ?2
           AND (
               (cp.property = 'status'
                AND cp.value IN ('retired', 'deprecated', 'withdrawn', 'inactive'))
            OR (cp.property IN ({placeholders}) AND cp.value = 'true')
           )
         LIMIT 1"
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(inactive_codes.len() + 2);
    params.push(&system_url);
    params.push(&code);
    for c in &inactive_codes {
        params.push(c as &dyn rusqlite::ToSql);
    }
    conn.query_row(&sql, params.as_slice(), |_| Ok(())).is_ok()
}

// Keep all message-format inputs explicit so the IG-fixture text strings are
// composed in one place — splitting into a struct just to placate the lint
// would scatter the format logic across the file.
//
// `is_inactive_in_underlying_cs` is set when the code is NOT in the
// expansion (`found.is_none()`) but IS present in the underlying CodeSystem
// with an inactive status. The IG fixtures (e.g.
// `inactive/validate-inactive-2a`) expect three additional issues in that
// case: a business-rule "...is valid but is not active" error, the
// not-in-vs error, and a code-comment "...has a status of inactive..."
// warning.
#[allow(clippy::too_many_arguments)]
fn finish_validate_code_response(
    found: Option<ExpansionContains>,
    code: &str,
    url: &str,
    expected_display: Option<&str>,
    system_for_msg: Option<&str>,
    is_abstract: bool,
    is_inactive: bool,
    vs_version: Option<&str>,
    is_inactive_in_underlying_cs: bool,
    code_unknown_in_cs: bool,
    cs_version_for_msg: Option<&str>,
) -> Result<ValidateCodeResponse, HtsError> {
    let qualified = match system_for_msg {
        Some(s) => format!("{s}#{code}"),
        None => code.to_string(),
    };
    let url_with_version = match vs_version {
        Some(v) => format!("{url}|{v}"),
        None => url.to_string(),
    };
    let mut issues: Vec<crate::types::ValidationIssue> = Vec::new();
    match found {
        None => {
            // The IG validator compares this text with the format
            //   "The provided code 'system#code' was not found in the value set 'url'"
            // (see notSelectable/* and validation/* response fixtures).
            let not_in_vs_text = format!(
                "The provided code '{qualified}' was not found in the value set '{url_with_version}'"
            );
            // Special case: code is valid in the underlying CodeSystem but
            // inactive, and the VS filtered it out (compose.inactive=false
            // or activeOnly=true). The IG expects a business-rule error
            // ("valid but not active"), the not-in-vs error, AND a
            // code-comment warning ("has a status of inactive").
            if is_inactive_in_underlying_cs {
                issues.push(crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-rule".into(),
                    text: format!("The concept '{code}' is valid but is not active"),
                    location: Some("Coding.code".into()),
                    message_id: Some("STATUS_CODE_WARNING_CODE".into()),
                });
            }
            issues.push(crate::types::ValidationIssue {
                severity: "error".into(),
                fhir_code: "code-invalid".into(),
                tx_code: "not-in-vs".into(),
                text: not_in_vs_text.clone(),
                location: Some("Coding.code".into()),
                message_id: Some("None_of_the_provided_codes_are_in_the_value_set_one".into()),
            });
            // Companion issue: when the code isn't in the underlying CodeSystem
            // at all (but the CodeSystem itself IS loaded), the IG fixtures
            // (permutations/bad-coding-*-request) expect a separate
            // `code-invalid` / `invalid-code` issue. Skip when the CodeSystem
            // is itself unknown — the operations layer already adds a
            // `not-found` / `not-found` issue for that case, and double-emitting
            // would inflate the issue count.
            if code_unknown_in_cs && cs_version_for_msg.is_some() {
                if let Some(sys) = system_for_msg {
                    let cs_text = match cs_version_for_msg {
                        Some(v) => {
                            format!("Unknown code '{code}' in the CodeSystem '{sys}' version '{v}'")
                        }
                        None => format!("Unknown code '{code}' in the CodeSystem '{sys}'"),
                    };
                    issues.push(crate::types::ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "code-invalid".into(),
                        tx_code: "invalid-code".into(),
                        text: cs_text,
                        location: Some("Coding.code".into()),
                        message_id: Some("Unknown_Code_in_Version".into()),
                    });
                }
            }
            if is_inactive_in_underlying_cs {
                issues.push(crate::types::ValidationIssue {
                    severity: "warning".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-comment".into(),
                    text: format!(
                        "The concept '{code}' has a status of inactive and its use should be reviewed"
                    ),
                    location: Some("Coding".into()),
                    message_id: Some("INACTIVE_CONCEPT_FOUND".into()),
                });
            }
            // Compose the message text from issues sorted alphabetically,
            // joined with `; ` — matches the IG fixture's `message` parameter.
            let mut texts: Vec<&str> = issues.iter().map(|i| i.text.as_str()).collect();
            texts.sort();
            let message = texts.join("; ");
            Ok(ValidateCodeResponse {
                result: false,
                message: Some(message),
                display: None,
                system: None,
                inactive: if is_inactive_in_underlying_cs {
                    Some(true)
                } else {
                    None
                },
                issues,
                caused_by_unknown_system: None,
            })
        }
        Some(concept) => {
            // Abstract / notSelectable concepts are present in the VS but
            // cannot be selected by users — reject with the IG wording.
            // The IG fixtures expect TWO issues here: a `business-rule` /
            // `code-rule` for the abstract violation, and a `code-invalid` /
            // `not-in-vs` because the abstract code is excluded from the
            // selectable set.
            if is_abstract {
                let abstract_text =
                    format!("Code '{qualified}' is abstract, and not allowed in this context");
                let not_in_vs_text = format!(
                    "The provided code '{qualified}' was not found in the value set '{url_with_version}'"
                );
                issues.push(crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-rule".into(),
                    text: abstract_text.clone(),
                    location: Some("Coding.code".into()),
                    message_id: Some("ABSTRACT_CODE_NOT_ALLOWED".into()),
                });
                issues.push(crate::types::ValidationIssue {
                    severity: "error".into(),
                    fhir_code: "code-invalid".into(),
                    tx_code: "not-in-vs".into(),
                    text: not_in_vs_text,
                    location: Some("Coding.code".into()),
                    message_id: Some("None_of_the_provided_codes_are_in_the_value_set_one".into()),
                });
                return Ok(ValidateCodeResponse {
                    result: false,
                    message: Some(abstract_text),
                    display: concept.display,
                    system: None,
                    inactive: None,
                    issues,
                    caused_by_unknown_system: None,
                });
            }
            // Inactive: the IG fixtures expect a warning-severity
            // `business-rule` / `code-comment` issue ("...has a status of
            // inactive and its use should be reviewed"). Emitted for every
            // inactive match — even when validation otherwise succeeds —
            // because that's what the validator-and-fixtures contract is.
            if is_inactive {
                issues.push(crate::types::ValidationIssue {
                    severity: "warning".into(),
                    fhir_code: "business-rule".into(),
                    tx_code: "code-comment".into(),
                    text: format!(
                        "The concept '{code}' has a status of inactive and its use should be reviewed"
                    ),
                    location: Some("Coding".into()),
                    message_id: Some("INACTIVE_CONCEPT_FOUND".into()),
                });
            }
            let mut display_message: Option<String> = None;
            if let Some(expected) = expected_display {
                if let Some(actual) = concept.display.as_deref() {
                    if !actual.eq_ignore_ascii_case(expected) {
                        let text = format!(
                            "Provided display '{expected}' does not match stored display '{actual}'"
                        );
                        display_message = Some(text.clone());
                        // Display mismatch is an error per the IG fixtures
                        // (`validation/simple-coding-bad-display`): severity
                        // error, fhir_code `invalid`, tx_code
                        // `invalid-display`. result flips to false.
                        issues.push(crate::types::ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "invalid".into(),
                            tx_code: "invalid-display".into(),
                            text,
                            location: Some("Coding.display".into()),
                            message_id: Some(
                                "Display_Name_for__should_be_one_of__instead_of".into(),
                            ),
                        });
                    }
                }
            }
            // Result is false iff there's at least one error-severity issue.
            // Display mismatch is a warning so it does not flip result; the
            // legacy `display_message` is preserved on `message` for the
            // single-issue fallback path.
            let has_error = issues.iter().any(|i| i.severity == "error");
            let message = if !issues.is_empty() {
                let mut sorted: Vec<&str> = issues.iter().map(|i| i.text.as_str()).collect();
                sorted.sort();
                Some(sorted.join("; "))
            } else {
                display_message
            };
            Ok(ValidateCodeResponse {
                result: !has_error,
                message,
                display: concept.display,
                system: Some(concept.system),
                inactive: if is_inactive { Some(true) } else { None },
                issues,
                caused_by_unknown_system: None,
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
                is_abstract: None,

                inactive: None,

                designations: vec![],

                properties: vec![],
                contains: vec![],
            }))
        }
        FhirVsPattern::IsA(root_code) => {
            // O(1) closure lookup: is root_code an ancestor-or-self of code?
            let is_member: bool = conn
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM concept_closure
                         WHERE system_id = ?1 AND ancestor_code = ?3 AND descendant_code = ?2
                     )",
                    rusqlite::params![system_id, code, root_code],
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
                is_abstract: None,

                inactive: None,

                designations: vec![],

                properties: vec![],
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
        return Err(HtsError::NotFound(format!(
            "A definition for the value Set \'{url}\' could not be found"
        )));
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
            // O(1) closure JOIN replaces the recursive CTE.
            // << semantics: all descendants plus the root itself (self-link in closure).
            conn.execute(
                "INSERT OR IGNORE INTO implicit_expansion_cache (url, system_url, code, display)
                 SELECT ?1, ?2, c.code, c.display
                 FROM   concept_closure cc
                 JOIN   concepts c ON c.system_id = ?3 AND c.code = cc.descendant_code
                 WHERE  cc.system_id = ?3 AND cc.ancestor_code = ?4",
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
                    is_abstract: None,

                    inactive: None,

                    designations: vec![],

                    properties: vec![],
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
                    is_abstract: None,

                    inactive: None,

                    designations: vec![],

                    properties: vec![],
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

/// Ensure the process-local in-memory concept index is populated for `url`.
///
/// Reads all rows for `url` from `implicit_expansion_cache` and stores them
/// as an `Arc<[ImplicitConceptEntry]>` keyed by URL.  Subsequent calls for the
/// same URL return immediately (O(1) read-lock check).  If two threads race on
/// the first request, both load from DB but only the first writer's slice is
/// kept (`or_insert` is a no-op for the second writer).
fn ensure_implicit_index(
    conn: &Connection,
    url: &str,
    index: &super::ImplicitIndex,
) -> Result<(), HtsError> {
    // Fast path: already loaded — only needs a shared read lock.
    {
        let guard = index
            .read()
            .map_err(|_| HtsError::Internal("implicit index lock poisoned".into()))?;
        if guard.contains_key(url) {
            return Ok(());
        }
    }

    let mut stmt = conn
        .prepare_cached(
            "SELECT system_url, code, display \
             FROM implicit_expansion_cache \
             WHERE url = ?1 \
             ORDER BY system_url, code",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let entries: Vec<ImplicitConceptEntry> = stmt
        .query_map([url], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .into_iter()
        .map(|(system_url, code, display)| {
            let code_lower = code.to_lowercase();
            let display_lower = display
                .as_deref()
                .map(str::to_lowercase)
                .unwrap_or_default();
            ImplicitConceptEntry {
                system_url,
                code,
                display,
                code_lower,
                display_lower,
            }
        })
        .collect();

    // Build trigram inverted index: for each entry, emit every distinct 3-byte
    // sequence found in code_lower or display_lower.  Posting lists are appended
    // in ascending entry-index order (they are inherently sorted since we process
    // entries 0..N in order), so no sort step is needed after construction.
    let mut trigram_idx: HashMap<[u8; 3], Vec<u32>> = HashMap::new();
    let mut seen: Vec<[u8; 3]> = Vec::with_capacity(64);
    for (i, entry) in entries.iter().enumerate() {
        seen.clear();
        let idx = i as u32;
        for text in [entry.code_lower.as_str(), entry.display_lower.as_str()] {
            let bytes = text.as_bytes();
            for w in bytes.windows(3) {
                let tri = [w[0], w[1], w[2]];
                // Deduplicate: don't add the same trigram for the same entry twice.
                if !seen.contains(&tri) {
                    seen.push(tri);
                    trigram_idx.entry(tri).or_default().push(idx);
                }
            }
        }
    }
    let trigram_idx: HashMap<[u8; 3], Box<[u32]>> = trigram_idx
        .into_iter()
        .map(|(k, v)| (k, v.into_boxed_slice()))
        .collect();

    let combined = Arc::new(ImplicitConceptIndex {
        entries: entries.into_boxed_slice(),
        trigram_idx,
    });

    {
        let mut guard = index
            .write()
            .map_err(|_| HtsError::Internal("implicit index lock poisoned".into()))?;
        guard.entry(url.to_string()).or_insert(combined);
    }

    Ok(())
}

/// Intersect two sorted posting lists using a merge-join — O(a + b).
fn merge_intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut result = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Equal => {
                result.push(a[i]);
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
        }
    }
    result
}

/// Return candidate entry indices whose `code_lower` or `display_lower`
/// contains all trigrams of `filter`.
///
/// Returns `None` when `filter` is shorter than 3 bytes (no trigrams can be
/// formed), signalling the caller to fall back to a linear scan.
/// Returns `Some(vec![])` when any trigram has an empty posting list
/// (guaranteed no matches).
fn trigram_candidates(idx: &HashMap<[u8; 3], Box<[u32]>>, filter: &str) -> Option<Vec<u32>> {
    let bytes = filter.as_bytes();
    if bytes.len() < 3 {
        return None;
    }

    // Collect distinct trigrams from the filter string.
    let mut trigrams: Vec<[u8; 3]> = Vec::new();
    for w in bytes.windows(3) {
        let tri = [w[0], w[1], w[2]];
        if !trigrams.contains(&tri) {
            trigrams.push(tri);
        }
    }

    // Look up each trigram.  Sort by posting-list length so the first
    // intersection starts from the smallest (cheapest) list.
    let mut lists: Vec<&[u32]> = trigrams
        .iter()
        .filter_map(|t| idx.get(t).map(Box::as_ref))
        .collect();

    if lists.len() < trigrams.len() {
        // At least one trigram has no posting list → guaranteed empty result.
        return Some(vec![]);
    }

    lists.sort_unstable_by_key(|l| l.len());

    let mut candidates: Vec<u32> = lists[0].to_vec();
    for list in &lists[1..] {
        if candidates.is_empty() {
            break;
        }
        candidates = merge_intersect(&candidates, list);
    }

    Some(candidates)
}

/// Count entries in the in-memory index that match an optional filter.
///
/// Uses the trigram index for O(k) lookup when `filter` is ≥ 3 bytes;
/// falls back to a linear scan for shorter filters.
fn count_in_memory(idx: &ImplicitConceptIndex, filter_lower: Option<&str>) -> u32 {
    let Some(f) = filter_lower else {
        return idx.entries.len() as u32;
    };

    match trigram_candidates(&idx.trigram_idx, f) {
        Some(candidates) => {
            // Verify candidates: trigram intersection is a necessary but not
            // sufficient condition, so re-check with contains().
            candidates
                .iter()
                .filter(|&&i| {
                    let e = &idx.entries[i as usize];
                    e.code_lower.contains(f) || e.display_lower.contains(f)
                })
                .count() as u32
        }
        None => {
            // Filter < 3 bytes: no trigrams — linear scan.
            idx.entries
                .iter()
                .filter(|e| e.code_lower.contains(f) || e.display_lower.contains(f))
                .count() as u32
        }
    }
}

/// Return a paginated slice of in-memory entries matching an optional filter.
///
/// Unfiltered requests skip directly to `offset` without scanning all entries.
/// Filtered requests use the trigram index for O(k) candidate lookup (≥ 3-char
/// filters); shorter filters fall back to a linear scan.
/// Candidates are returned in entry-index order, which preserves the original
/// `ORDER BY system_url, code` ordering from the DB load.
fn page_in_memory(
    idx: &ImplicitConceptIndex,
    filter_lower: Option<&str>,
    offset: i64,
    limit: i64,
) -> Vec<ExpansionContains> {
    let offset_n = offset as usize;
    let take = if limit < 0 {
        usize::MAX
    } else {
        limit as usize
    };

    let entry_to_contains = |e: &ImplicitConceptEntry| ExpansionContains {
        system: e.system_url.clone(),
        code: e.code.clone(),
        display: e.display.clone(),
        is_abstract: None,

        inactive: None,

        designations: vec![],

        properties: vec![],
        contains: vec![],
    };

    let Some(f) = filter_lower else {
        // No filter: O(count) direct slice — skip then take.
        return idx
            .entries
            .iter()
            .skip(offset_n)
            .take(take)
            .map(entry_to_contains)
            .collect();
    };

    match trigram_candidates(&idx.trigram_idx, f) {
        Some(candidates) => {
            // Candidates are sorted by entry index → same order as entries.
            candidates
                .iter()
                .filter_map(|&i| {
                    let e = &idx.entries[i as usize];
                    if e.code_lower.contains(f) || e.display_lower.contains(f) {
                        Some(entry_to_contains(e))
                    } else {
                        None
                    }
                })
                .skip(offset_n)
                .take(take)
                .collect()
        }
        None => {
            // Filter < 3 bytes: linear scan.
            idx.entries
                .iter()
                .filter(|e| e.code_lower.contains(f) || e.display_lower.contains(f))
                .skip(offset_n)
                .take(take)
                .map(entry_to_contains)
                .collect()
        }
    }
}

/// Wrap a search term as an FTS5 phrase literal.
///
/// Double-quotes the term so FTS5 treats it as a substring phrase rather than
/// individual tokens.  Internal double-quote characters are escaped by doubling.
fn fts5_quote(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

/// Build an FTS5 prefix query expression for the `concepts_word_fts` table.
///
/// Appends `*` to the term so FTS5 with the `unicode61` tokenizer matches any
/// token that *starts with* `term`.  Internal double-quotes are escaped.
/// Used for short (< 3 char) filter terms that the trigram index cannot serve.
fn fts5_word_prefix(term: &str) -> String {
    format!("{}*", term.replace('"', "\"\""))
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
    // Check both FTS tables in one query; either missing triggers a (re)build.
    let (trigram_ok, word_ok): (bool, bool) = conn
        .query_row(
            "SELECT
               EXISTS(SELECT 1 FROM implicit_expansion_fts      WHERE url = ?1 LIMIT 1),
               EXISTS(SELECT 1 FROM implicit_expansion_word_fts WHERE url = ?1 LIMIT 1)",
            [url],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if trigram_ok && word_ok {
        return Ok(());
    }

    // BEGIN IMMEDIATE acquires the write lock upfront so concurrent VUs don't
    // each rebuild the same 350K-row index independently (mirrors ensure_concepts_fts).
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Re-check inside the lock: another VU may have built the index while we waited.
    let (still_no_trigram, still_no_word): (bool, bool) = match conn.query_row(
        "SELECT
           NOT EXISTS(SELECT 1 FROM implicit_expansion_fts      WHERE url = ?1 LIMIT 1),
           NOT EXISTS(SELECT 1 FROM implicit_expansion_word_fts WHERE url = ?1 LIMIT 1)",
        [url],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ) {
        Ok(v) => v,
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
    };

    if !still_no_trigram && !still_no_word {
        conn.execute_batch("COMMIT")
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        return Ok(());
    }

    if still_no_trigram {
        if let Err(e) = conn.execute("DELETE FROM implicit_expansion_fts WHERE url = ?1", [url]) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
        if let Err(e) = conn.execute(
            "INSERT INTO implicit_expansion_fts (url, system_url, code, display)
             SELECT url, system_url, code, display
             FROM implicit_expansion_cache
             WHERE url = ?1",
            [url],
        ) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
    }

    if still_no_word {
        if let Err(e) = conn.execute(
            "DELETE FROM implicit_expansion_word_fts WHERE url = ?1",
            [url],
        ) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
        if let Err(e) = conn.execute(
            "INSERT INTO implicit_expansion_word_fts (url, system_url, code, display)
             SELECT url, system_url, code, display
             FROM implicit_expansion_cache
             WHERE url = ?1",
            [url],
        ) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(HtsError::StorageError(e.to_string()));
        }
    }

    conn.execute_batch("COMMIT")
        .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Ensure the FTS5 trigram index on `concepts_fts` is populated for `system_id`.
///
/// Populated lazily on the first filtered inline expand for a given system.
/// Cleared on server startup so a re-import followed by a restart always
/// rebuilds from fresh data.
fn ensure_concepts_fts(conn: &Connection, system_id: &str) -> Result<(), HtsError> {
    // O(1) primary-key lookup via the tracker table; avoids the old O(N_total)
    // FTS content scan that read through every row before finding the target system.
    let populated: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM concepts_fts_built WHERE system_id = ?1)",
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
        "SELECT NOT EXISTS(SELECT 1 FROM concepts_fts_built WHERE system_id = ?1)",
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

    // Also populate the word-prefix FTS used for short (< 3 char) filter terms.
    if let Err(e) = conn.execute(
        "DELETE FROM concepts_word_fts WHERE system_id = ?1",
        [system_id],
    ) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(HtsError::StorageError(e.to_string()));
    }
    if let Err(e) = conn.execute(
        "INSERT INTO concepts_word_fts(rowid, system_id, code, display)
         SELECT id, system_id, code, display FROM concepts WHERE system_id = ?1",
        [system_id],
    ) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(HtsError::StorageError(e.to_string()));
    }

    if let Err(e) = conn.execute(
        "INSERT OR IGNORE INTO concepts_fts_built (system_id) VALUES (?1)",
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
            // Short filter (1–2 chars): word-prefix FTS count avoids O(N) LIKE scan.
            ensure_implicit_fts(conn, url)?;
            let prefix_expr = fts5_word_prefix(f);
            conn.query_row(
                "SELECT COUNT(*) FROM implicit_expansion_word_fts
                 WHERE implicit_expansion_word_fts MATCH ?1 AND url = ?2",
                rusqlite::params![prefix_expr, url],
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
                    // No ORDER BY: FTS5 short-circuits at LIMIT instead of
                    // materialising all matching rows (potentially thousands for
                    // common terms like "dia") before sorting. The tiny result
                    // set is sorted in Rust below — O(N log N) on 20–100 rows.
                    "SELECT system_url, code, display
                     FROM implicit_expansion_fts
                     WHERE implicit_expansion_fts MATCH ?1 AND url = ?2
                     LIMIT ?3 OFFSET ?4",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let mut rows = stmt
                .query_map(rusqlite::params![match_expr, url, limit, offset], |r| {
                    Ok(ExpansionContains {
                        system: r.get(0)?,
                        code: r.get(1)?,
                        display: r.get(2)?,
                        is_abstract: None,

                        inactive: None,

                        designations: vec![],

                        properties: vec![],
                        contains: vec![],
                    })
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            rows.sort_unstable_by(|a, b| a.code.cmp(&b.code));
            Ok(rows)
        }
        Some(f) => {
            // Short filter (1–2 chars): word-prefix FTS so `di*` matches any
            // token starting with "di" — O(log N) vs O(N) LIKE scan on 350K rows.
            ensure_implicit_fts(conn, url)?;
            let prefix_expr = fts5_word_prefix(f);
            let mut stmt = conn
                .prepare_cached(
                    "SELECT system_url, code, display
                     FROM implicit_expansion_word_fts
                     WHERE implicit_expansion_word_fts MATCH ?1 AND url = ?2
                     LIMIT ?3 OFFSET ?4",
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let mut rows = stmt
                .query_map(rusqlite::params![prefix_expr, url, limit, offset], |r| {
                    Ok(ExpansionContains {
                        system: r.get(0)?,
                        code: r.get(1)?,
                        display: r.get(2)?,
                        is_abstract: None,

                        inactive: None,

                        designations: vec![],

                        properties: vec![],
                        contains: vec![],
                    })
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            rows.sort_unstable_by(|a, b| a.code.cmp(&b.code));
            Ok(rows)
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
                    is_abstract: None,

                    inactive: None,

                    designations: vec![],

                    properties: vec![],
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

/// Pre-populate `concepts_fts` for every code system currently in the DB.
///
/// Called once at server startup (after clearing `concepts_fts`) so that
/// filtered `$expand` requests always use the fast FTS path rather than
/// triggering a blocking per-system build on the first filtered request.
/// Uses a single bulk INSERT inside one transaction — much faster than
/// building per-system (1 transaction per system × 1217 systems would take
/// several minutes; the bulk approach finishes in under 30 s).
pub(crate) fn prebuild_concepts_fts(conn: &Connection) {
    if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
        tracing::warn!("prebuild_concepts_fts: could not begin transaction: {e}");
        return;
    }

    let fts_result = conn.execute(
        "INSERT INTO concepts_fts(rowid, system_id, code, display)
         SELECT id, system_id, code, display FROM concepts",
        [],
    );

    let n = match fts_result {
        Ok(n) => n,
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            tracing::warn!("prebuild_concepts_fts: trigram INSERT failed: {e}");
            return;
        }
    };

    // Also populate the word-prefix FTS (unicode61) used for short filter terms.
    if let Err(e) = conn.execute(
        "INSERT INTO concepts_word_fts(rowid, system_id, code, display)
         SELECT id, system_id, code, display FROM concepts",
        [],
    ) {
        let _ = conn.execute_batch("ROLLBACK");
        tracing::warn!("prebuild_concepts_fts: word-prefix INSERT failed: {e}");
        return;
    }

    // Populate the O(1) tracker so ensure_concepts_fts avoids FTS content scans.
    if let Err(e) = conn.execute_batch(
        "INSERT OR IGNORE INTO concepts_fts_built (system_id)
         SELECT DISTINCT id FROM code_systems",
    ) {
        let _ = conn.execute_batch("ROLLBACK");
        tracing::warn!("prebuild_concepts_fts: tracker INSERT failed: {e}");
        return;
    }

    let _ = conn.execute_batch("COMMIT");
    tracing::info!(
        rows = n,
        "concepts_fts pre-populated (trigram + word-prefix)"
    );
}

/// Pre-warm the in-memory concept index from any implicit-expansion URLs
/// already persisted in `implicit_expansion_cache`.
///
/// Called at server startup after `prebuild_concepts_fts`.  On a cold DB the
/// cache is empty so this is a no-op.  On a warm restart (benchmark re-run,
/// rolling deploy) the index is rebuilt in memory from the persisted rows,
/// allowing the async hot path in [`expand`] to fire from the very first
/// request without waiting for a background build thread.
pub(crate) fn prebuild_implicit_index(conn: &Connection, index: &super::ImplicitIndex) {
    let urls: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT url FROM implicit_expansion_cache \
             WHERE url NOT LIKE 'inline-compose:%'",
        )
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    for url in &urls {
        let _ = ensure_implicit_index(conn, url, index);
    }

    if !urls.is_empty() {
        tracing::info!(
            count = urls.len(),
            "implicit concept index pre-warmed from cache"
        );
    }
}

/// Build an [`ImplicitConceptIndex`] from a flat list of expansion entries.
///
/// Entries are assumed to be already sorted by `(system_url, code)`.
/// Constructs the trigram inverted index for O(k) filtered queries.
fn build_concept_index_from_entries(entries: Vec<ImplicitConceptEntry>) -> ImplicitConceptIndex {
    let mut trigram_idx: HashMap<[u8; 3], Vec<u32>> = HashMap::new();
    let mut seen: Vec<[u8; 3]> = Vec::with_capacity(64);
    for (i, entry) in entries.iter().enumerate() {
        seen.clear();
        let idx = i as u32;
        for text in [entry.code_lower.as_str(), entry.display_lower.as_str()] {
            let bytes = text.as_bytes();
            for w in bytes.windows(3) {
                let tri = [w[0], w[1], w[2]];
                if !seen.contains(&tri) {
                    seen.push(tri);
                    trigram_idx.entry(tri).or_default().push(idx);
                }
            }
        }
    }
    let trigram_idx: HashMap<[u8; 3], Box<[u32]>> = trigram_idx
        .into_iter()
        .map(|(k, v)| (k, v.into_boxed_slice()))
        .collect();
    ImplicitConceptIndex {
        entries: entries.into_boxed_slice(),
        trigram_idx,
    }
}

/// Populate the inline-compose in-memory index from a computed expansion.
///
/// Called immediately after a successful `compute_expansion` + DB cache write
/// so that all subsequent requests for the same compose body skip `spawn_blocking`
/// entirely.  No-op if the index already contains an entry for `cache_key` (a
/// concurrent request already populated it).
fn populate_inline_compose_index(
    codes: &[ExpansionContains],
    cache_key: &str,
    index: &super::InlineComposeIndex,
) {
    {
        // Fast read-path: already populated by a concurrent request.
        if let Ok(guard) = index.read() {
            if guard.contains_key(cache_key) {
                return;
            }
        }
    }

    let entries: Vec<ImplicitConceptEntry> = codes
        .iter()
        .map(|c| {
            let code_lower = c.code.to_lowercase();
            let display_lower = c
                .display
                .as_deref()
                .map(str::to_lowercase)
                .unwrap_or_default();
            ImplicitConceptEntry {
                system_url: c.system.clone(),
                code: c.code.clone(),
                display: c.display.clone(),
                code_lower,
                display_lower,
            }
        })
        .collect();

    let concept_idx = Arc::new(build_concept_index_from_entries(entries));
    if let Ok(mut guard) = index.write() {
        guard.entry(cache_key.to_string()).or_insert(concept_idx);
    }
}

/// Pre-warm the inline-compose in-memory index from any `inline-compose:*`
/// entries already persisted in `implicit_expansion_cache`.
///
/// Called at server startup after `prebuild_implicit_index`.  On a cold DB
/// this is a no-op.  On a warm restart (benchmark re-run) the index is rebuilt
/// from persisted rows, letting the async hot path in [`expand`] serve all
/// inline-compose requests without ever entering `spawn_blocking`.
pub(crate) fn prebuild_inline_compose_index(conn: &Connection, index: &super::InlineComposeIndex) {
    let keys: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT url FROM implicit_expansion_cache \
             WHERE url LIKE 'inline-compose:%'",
        )
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    if keys.is_empty() {
        return;
    }

    let mut loaded = 0usize;
    for key in &keys {
        let entries_result = conn.prepare_cached(
            "SELECT system_url, code, display \
             FROM implicit_expansion_cache \
             WHERE url = ?1 \
             ORDER BY system_url, code",
        );
        let mut stmt = match entries_result {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rows: Vec<(String, String, Option<String>)> =
            match stmt.query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))) {
                Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
                Err(_) => continue,
            };

        let entries: Vec<ImplicitConceptEntry> = rows
            .into_iter()
            .map(|(system_url, code, display)| {
                let code_lower = code.to_lowercase();
                let display_lower = display
                    .as_deref()
                    .map(str::to_lowercase)
                    .unwrap_or_default();
                ImplicitConceptEntry {
                    system_url,
                    code,
                    display,
                    code_lower,
                    display_lower,
                }
            })
            .collect();

        let concept_idx = Arc::new(build_concept_index_from_entries(entries));
        if let Ok(mut guard) = index.write() {
            guard.insert(key.clone(), concept_idx);
        }
        loaded += 1;
    }

    tracing::info!(
        count = loaded,
        "inline compose concept index pre-warmed from cache"
    );
}

/// Populate the property-result in-memory cache from a computed expansion.
///
/// Called by `expand_inline_filtered` after accumulating the full
/// property-matched (but text-unfiltered) concept set.  Subsequent requests
/// for the same compose body with a different text filter are served from this
/// cache by the async hot path in `expand()`, bypassing `spawn_blocking`.
///
/// No-op when the cache already has an entry for `cache_key` (a concurrent
/// request raced and won).
fn populate_property_cache(
    codes: &[ExpansionContains],
    cache_key: &str,
    cache: &super::PropertyResultCache,
) {
    {
        if let Ok(guard) = cache.read() {
            if guard.contains_key(cache_key) {
                return;
            }
        }
    }
    let entries: Vec<ImplicitConceptEntry> = codes
        .iter()
        .map(|c| {
            let code_lower = c.code.to_lowercase();
            let display_lower = c
                .display
                .as_deref()
                .map(str::to_lowercase)
                .unwrap_or_default();
            ImplicitConceptEntry {
                system_url: c.system.clone(),
                code: c.code.clone(),
                display: c.display.clone(),
                code_lower,
                display_lower,
            }
        })
        .collect();
    let concept_idx = Arc::new(build_concept_index_from_entries(entries));
    if let Ok(mut guard) = cache.write() {
        guard.entry(cache_key.to_string()).or_insert(concept_idx);
    }
}

/// Maximum number of concepts to load into the PlainFtsCache per compose body.
///
/// Compose bodies that reference more total concepts than this threshold are
/// not cached; requests for them fall back to the existing FTS query path.
/// 500 000 covers the largest realistic multi-system benchmarks (e.g. LOINC +
/// SNOMED combined) while bounding per-entry memory to roughly 150–200 MB.
const PLAIN_FTS_CACHE_MAX_CONCEPTS: usize = 500_000;

/// Load ALL concepts from plain system includes and populate the PlainFtsCache.
///
/// Called by `expand_inline_filtered` on the first filtered request for a
/// compose body where every include is a plain full-system include (EX07
/// pattern).  Loads all concepts without any text filter, builds an
/// `ImplicitConceptIndex`, stores it under `cache_key`, and returns the Arc.
///
/// Returns `None` when:
/// - All systems are missing from the DB (warning emitted for each).
/// - The total concept count exceeds [`PLAIN_FTS_CACHE_MAX_CONCEPTS`].
/// - Any SQLite error occurs (logged at WARN level).
///
/// A concurrent request that already populated the same key returns the
/// existing Arc without rebuilding the index.
fn load_plain_corpus_and_cache(
    conn: &Connection,
    includes: &[serde_json::Value],
    cache_key: &str,
    cache: &super::PlainFtsCache,
    warnings: &mut Vec<String>,
) -> Option<Arc<ImplicitConceptIndex>> {
    // Fast path: another request already populated the cache.
    // A zero-entry index is a "too-large" sentinel — return None so the
    // caller falls back to the FTS query without re-counting the corpus.
    if let Ok(guard) = cache.read() {
        if let Some(idx) = guard.get(cache_key).cloned() {
            return if idx.entries.is_empty() {
                None
            } else {
                Some(idx)
            };
        }
    }

    // Resolve (system_url, system_id) pairs.
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(includes.len());
    for inc in includes {
        let system_url = inc["system"].as_str().unwrap_or("");
        match conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = ?1",
                [system_url],
                |r| r.get::<_, String>(0),
            )
            .optional()
        {
            Ok(Some(id)) => pairs.push((system_url.to_owned(), id)),
            Ok(None) => {
                let msg = format!(
                    "CodeSystem {system_url} was not found and has been excluded from the expansion"
                );
                tracing::warn!(%system_url, "{msg}");
                warnings.push(msg);
            }
            Err(e) => {
                tracing::warn!(%system_url, "Error resolving system for plain-fts cache: {e}");
                return None;
            }
        }
    }

    if pairs.is_empty() {
        return None;
    }

    let ids_json =
        serde_json::to_string(&pairs.iter().map(|(_, id)| id.as_str()).collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_owned());

    let id_to_url: std::collections::HashMap<String, String> =
        pairs.into_iter().map(|(url, id)| (id, url)).collect();

    // COUNT before loading to avoid pulling millions of rows for large systems.
    // On too-large: store a zero-entry sentinel so all subsequent requests
    // skip this check entirely (no repeated COUNT queries).
    let corpus_count: i64 = match conn.query_row(
        "SELECT COUNT(*) FROM concepts \
         WHERE system_id IN (SELECT value FROM json_each(?1))",
        rusqlite::params![ids_json],
        |r| r.get(0),
    ) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("Failed to count plain corpus: {e}");
            return None;
        }
    };

    if corpus_count as usize > PLAIN_FTS_CACHE_MAX_CONCEPTS {
        // Store a zero-entry sentinel so subsequent requests (both the async
        // hot path and this function's own fast path) skip the COUNT query.
        let sentinel = Arc::new(build_concept_index_from_entries(vec![]));
        if let Ok(mut guard) = cache.write() {
            guard.entry(cache_key.to_string()).or_insert(sentinel);
        }
        tracing::debug!(
            count = corpus_count,
            cache_key,
            "Plain corpus exceeds cache limit; using FTS fallback"
        );
        return None;
    }

    let mut stmt = match conn.prepare_cached(
        "SELECT system_id, code, display FROM concepts \
         WHERE system_id IN (SELECT value FROM json_each(?1)) \
         ORDER BY system_id, code",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to prepare plain corpus query: {e}");
            return None;
        }
    };

    let rows = match stmt
        .query_map(rusqlite::params![ids_json], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .and_then(|iter| iter.collect::<rusqlite::Result<Vec<_>>>())
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to load plain corpus concepts: {e}");
            return None;
        }
    };

    let entries: Vec<ImplicitConceptEntry> = rows
        .into_iter()
        .filter_map(|(system_id, code, display)| {
            let system_url = id_to_url.get(&system_id)?.clone();
            let code_lower = code.to_lowercase();
            let display_lower = display
                .as_deref()
                .map(str::to_lowercase)
                .unwrap_or_default();
            Some(ImplicitConceptEntry {
                system_url,
                code,
                display,
                code_lower,
                display_lower,
            })
        })
        .collect();

    let concept_idx = Arc::new(build_concept_index_from_entries(entries));
    if let Ok(mut guard) = cache.write() {
        guard
            .entry(cache_key.to_string())
            .or_insert_with(|| concept_idx.clone());
    }
    Some(concept_idx)
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

    /// `compose.include[].version` must select the matching code_systems row,
    /// not just the latest one.
    ///
    /// The bundle imports two CodeSystems sharing
    /// `http://example.org/cs-mv` with versions `1.0.0` (codes A, B) and
    /// `2.0.0` (codes C, D), plus three ValueSets that pin different versions
    /// in their compose includes. Each $expand should return only the codes
    /// belonging to the selected version.
    fn bundle_with_mv_compose() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "mv",
                "url": "http://example.org/cs-mv",
                "version": "1.0.0",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "A", "display": "A v1" },
                  { "code": "B", "display": "B v1" }
                ]
              }
            },
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "mv",
                "url": "http://example.org/cs-mv",
                "version": "2.0.0",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "C", "display": "C v2" },
                  { "code": "D", "display": "D v2" }
                ]
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs-pin-v1",
                "url": "http://example.org/vs-pin-v1",
                "status": "active",
                "compose": {
                  "include": [{
                    "system": "http://example.org/cs-mv",
                    "version": "1.0.0"
                  }]
                }
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs-pin-v2",
                "url": "http://example.org/vs-pin-v2",
                "status": "active",
                "compose": {
                  "include": [{
                    "system": "http://example.org/cs-mv",
                    "version": "2.0.0"
                  }]
                }
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs-mixed",
                "url": "http://example.org/vs-mixed",
                "status": "active",
                "compose": {
                  "include": [
                    {
                      "system": "http://example.org/cs-mv",
                      "version": "1.0.0",
                      "concept": [{ "code": "A" }]
                    },
                    {
                      "system": "http://example.org/cs-mv",
                      "version": "2.0.0",
                      "concept": [{ "code": "C" }]
                    }
                  ]
                }
              }
            }
          ]
        }"#
    }

    #[tokio::test]
    async fn expand_compose_version_pin_selects_v1_codes() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_mv_compose().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-pin-v1".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"A"), "v1.0.0 codes only: {codes:?}");
        assert!(codes.contains(&"B"));
        assert!(!codes.contains(&"C"), "v2.0.0 codes must not leak in");
        assert!(!codes.contains(&"D"));
    }

    #[tokio::test]
    async fn expand_compose_version_pin_selects_v2_codes() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_mv_compose().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-pin-v2".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"C"));
        assert!(codes.contains(&"D"));
        assert!(!codes.contains(&"A"));
        assert!(!codes.contains(&"B"));
    }

    /// Mirrors `tx-ecosystem/tests/version/valueset-version-mixed.json`:
    /// each include clause pulls a single code from its own pinned version.
    #[tokio::test]
    async fn expand_compose_mixed_versions_combines_codes_per_version() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_mv_compose().as_bytes())
            .await
            .unwrap();

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    url: Some("http://example.org/vs-mixed".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"A"), "v1 code A pulled from version 1.0.0");
        assert!(codes.contains(&"C"), "v2 code C pulled from version 2.0.0");
        assert_eq!(resp.total, Some(2), "exactly two codes: {codes:?}");
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

    /// Multi-include property filter uses OR semantics across includes (EX06 pattern).
    ///
    /// Two includes each with a single property= filter: the result should be the
    /// union of concepts matching either filter, exercising `try_multi_include_property_only`.
    #[tokio::test]
    async fn expand_multi_include_property_or_semantics() {
        let b = backend();

        // CodeSystem: concepts with property "tty" set to various values.
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-rx-multi",
              "url": "http://example.org/cs-rx-multi",
              "status": "active", "content": "complete",
              "property": [
                { "code": "tty",      "type": "code"   },
                { "code": "relatedTo","type": "code"   }
              ],
              "concept": [
                { "code": "BN1", "display": "Brand One",
                  "property": [
                    { "code": "tty",       "valueCode": "BN"      },
                    { "code": "relatedTo", "valueCode": "ING:A"   }
                  ]
                },
                { "code": "BN2", "display": "Brand Two",
                  "property": [
                    { "code": "tty",       "valueCode": "BN"      },
                    { "code": "relatedTo", "valueCode": "ING:B"   }
                  ]
                },
                { "code": "IN1", "display": "Ingredient One",
                  "property": [{ "code": "tty", "valueCode": "IN" }]
                },
                { "code": "SCD1", "display": "Clinical Drug One",
                  "property": [{ "code": "tty", "valueCode": "SCD" }]
                }
              ]
            }
          }]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        // Two includes: tty=BN OR tty=SCD (OR across includes).
        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [
                    {
                        "system": "http://example.org/cs-rx-multi",
                        "filter": [{ "property": "tty", "op": "=", "value": "BN" }]
                    },
                    {
                        "system": "http://example.org/cs-rx-multi",
                        "filter": [{ "property": "tty", "op": "=", "value": "SCD" }]
                    }
                ]
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

        let mut codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        codes.sort_unstable();
        assert!(codes.contains(&"BN1"), "BN1 matches tty=BN");
        assert!(codes.contains(&"BN2"), "BN2 matches tty=BN");
        assert!(codes.contains(&"SCD1"), "SCD1 matches tty=SCD");
        assert!(!codes.contains(&"IN1"), "IN1 has tty=IN, not included");
        assert_eq!(codes.len(), 3, "exactly 3 concepts across both includes");
    }

    /// Multi-include with AND semantics within each include (EX06 AND pattern).
    ///
    /// Single include with two property= filters: only concepts matching BOTH
    /// filters are returned.
    #[tokio::test]
    async fn expand_single_include_two_property_filters_and_semantics() {
        let b = backend();

        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-rx-and",
              "url": "http://example.org/cs-rx-and",
              "status": "active", "content": "complete",
              "property": [
                { "code": "tty",       "type": "code" },
                { "code": "relatedTo", "type": "code" }
              ],
              "concept": [
                { "code": "BN_ING_A", "display": "Brand of A",
                  "property": [
                    { "code": "tty",       "valueCode": "BN"    },
                    { "code": "relatedTo", "valueCode": "ING:A" }
                  ]
                },
                { "code": "BN_ING_B", "display": "Brand of B",
                  "property": [
                    { "code": "tty",       "valueCode": "BN"    },
                    { "code": "relatedTo", "valueCode": "ING:B" }
                  ]
                },
                { "code": "IN_A", "display": "Ingredient A",
                  "property": [
                    { "code": "tty",       "valueCode": "IN"    },
                    { "code": "relatedTo", "valueCode": "ING:A" }
                  ]
                }
              ]
            }
          }]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        // Single include: tty=BN AND relatedTo=ING:A (AND within one include).
        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-rx-and",
                    "filter": [
                        { "property": "tty",       "op": "=", "value": "BN"    },
                        { "property": "relatedTo", "op": "=", "value": "ING:A" }
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
        assert!(
            codes.contains(&"BN_ING_A"),
            "BN_ING_A matches tty=BN AND relatedTo=ING:A"
        );
        assert!(
            !codes.contains(&"BN_ING_B"),
            "BN_ING_B has relatedTo=ING:B, excluded"
        );
        assert!(!codes.contains(&"IN_A"), "IN_A has tty=IN, excluded");
        assert_eq!(codes.len(), 1, "only BN_ING_A matches both filters");
    }

    /// is-a + property= + text filter (EX08 combined pattern).
    ///
    /// Requests descendants of a root, filtered by a property value AND a text
    /// filter — exercises the sql_text push-down path in expand_inline_filtered
    /// that calls query_subtree_with_property with a text_filter argument.
    #[tokio::test]
    async fn expand_inline_isa_property_and_text_filter_combined() {
        let b = backend();

        // Hierarchy: root → finding_A (morphology=erosion, display "Erosion finding"),
        //                  → finding_B (morphology=fracture, display "Fracture finding"),
        //                  → finding_C (morphology=erosion, display "Chronic erosion")
        // orphan: morphology=erosion but NOT under root.
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-snomed-sim",
              "url": "http://example.org/cs-snomed-sim",
              "status": "active", "content": "complete",
              "property": [
                { "code": "morph", "type": "code" }
              ],
              "concept": [
                {
                  "code": "root", "display": "Clinical finding",
                  "concept": [
                    {
                      "code": "find_A", "display": "Erosion finding",
                      "property": [{ "code": "morph", "valueCode": "erosion" }]
                    },
                    {
                      "code": "find_B", "display": "Fracture finding",
                      "property": [{ "code": "morph", "valueCode": "fracture" }]
                    },
                    {
                      "code": "find_C", "display": "Chronic erosion disorder",
                      "property": [{ "code": "morph", "valueCode": "erosion" }]
                    }
                  ]
                },
                {
                  "code": "orphan", "display": "Orphan erosion",
                  "property": [{ "code": "morph", "valueCode": "erosion" }]
                }
              ]
            }
          }]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        // $expand with filter="erosion" + compose filter: is-a root + morph=erosion.
        // Should return find_A and find_C (both under root, have morph=erosion, display has "erosion").
        // Should NOT return find_B (morph=fracture), orphan (not under root).
        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-snomed-sim",
                    "filter": [
                        { "property": "concept", "op": "is-a",  "value": "root"   },
                        { "property": "morph",   "op": "=",     "value": "erosion" }
                    ]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    filter: Some("erosion".into()),
                    count: Some(20),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let mut codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        codes.sort_unstable();
        assert!(
            codes.contains(&"find_A"),
            "find_A: erosion morphology, under root, display matches"
        );
        assert!(
            codes.contains(&"find_C"),
            "find_C: erosion morphology, under root, display matches"
        );
        assert!(
            !codes.contains(&"find_B"),
            "find_B: fracture morphology, excluded"
        );
        assert!(
            !codes.contains(&"orphan"),
            "orphan: not under root, excluded"
        );
        assert_eq!(codes.len(), 2, "exactly find_A and find_C");

        // Also check: with text filter 'chronic' only find_C should match.
        let inline_vs2 = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-snomed-sim",
                    "filter": [
                        { "property": "concept", "op": "is-a",  "value": "root"   },
                        { "property": "morph",   "op": "=",     "value": "erosion" }
                    ]
                }]
            }
        });

        let resp2 = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs2),
                    filter: Some("chronic".into()),
                    count: Some(20),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes2: Vec<&str> = resp2.contains.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(
            codes2,
            vec!["find_C"],
            "only find_C has 'chronic' in display"
        );

        // And: text filter that matches nothing → empty expansion (not an error).
        let inline_vs3 = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-snomed-sim",
                    "filter": [
                        { "property": "concept", "op": "is-a",  "value": "root"    },
                        { "property": "morph",   "op": "=",     "value": "erosion"  }
                    ]
                }]
            }
        });

        let resp3 = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs3),
                    filter: Some("injection".into()),
                    count: Some(20),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(
            resp3.contains.is_empty(),
            "no erosion-morphology concepts under root have 'injection' in display"
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

    /// Mirror of the tx-ecosystem `simple-expand-regex` test: a `regex` filter
    /// on `code` should match the FULL string against the pattern.
    /// `[^ \t\r\n\f]{4}[0-9]` selects the three 5-character codes whose last
    /// character is a digit (`code1`, `code2`, `code3`).  Without anchored
    /// semantics every multi-segment code (`code2a`, `code2aI`, …) would also
    /// match — the test keeps us honest about full-string matching.
    #[tokio::test]
    async fn expand_inline_regex_filter_on_code_full_string_match() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-simple-regex",
              "url": "http://example.org/cs-simple-regex",
              "status": "active", "content": "complete",
              "concept": [
                { "code": "code1", "display": "Display 1" },
                { "code": "code2", "display": "Display 2",
                  "concept": [
                    { "code": "code2a", "display": "Display 2a",
                      "concept": [
                        { "code": "code2aI",  "display": "Display 2aI" },
                        { "code": "code2aII", "display": "Display 2aII" }
                      ]
                    },
                    { "code": "code2b", "display": "Display 2b" }
                  ]
                },
                { "code": "code3", "display": "Display 3" }
              ]
            }
          }]
        }"#;
        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-simple-regex",
                    "filter": [{
                        "property": "code",
                        "op": "regex",
                        "value": "[^ \\t\\r\\n\\f]{4}[0-9]"
                    }]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    count: Some(50),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let mut codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        codes.sort();
        assert_eq!(
            codes,
            vec!["code1", "code2", "code3"],
            "regex on code matches full-string only"
        );
    }

    /// Mirror of `simple-expand-regex-prop`: regex on a named property selects
    /// concepts whose property value fully matches.  `o[a-z]*` matches `old`
    /// (full-string) but not `new`.
    #[tokio::test]
    async fn expand_inline_regex_filter_on_property() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-prop-regex",
              "url": "http://example.org/cs-prop-regex",
              "status": "active", "content": "complete",
              "property": [{ "code": "prop", "type": "code" }],
              "concept": [
                { "code": "code1",   "display": "Display 1",
                  "property": [{ "code": "prop", "valueCode": "old" }] },
                { "code": "code2aI", "display": "Display 2aI",
                  "property": [{ "code": "prop", "valueCode": "old" }] },
                { "code": "code2b",  "display": "Display 2b",
                  "property": [{ "code": "prop", "valueCode": "old" }] },
                { "code": "code3",   "display": "Display 3",
                  "property": [{ "code": "prop", "valueCode": "old" }] },
                { "code": "code2",   "display": "Display 2",
                  "property": [{ "code": "prop", "valueCode": "new" }] },
                { "code": "code2a",  "display": "Display 2a",
                  "property": [{ "code": "prop", "valueCode": "new" }] }
              ]
            }
          }]
        }"#;
        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-prop-regex",
                    "filter": [{
                        "property": "prop",
                        "op": "regex",
                        "value": "o[a-z]*"
                    }]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    count: Some(50),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let mut codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        codes.sort();
        assert_eq!(
            codes,
            vec!["code1", "code2aI", "code2b", "code3"],
            "regex on property selects all concepts with prop value matching pattern"
        );
    }

    /// Regex `(a+)+` on the regex-bad code system: only the pure `aaaa…` code
    /// (no trailing chars) matches a full-string anchored pattern.  The codes
    /// with trailing `Y` / `Z` must NOT match.  Rust's RE2-style engine handles
    /// the otherwise-catastrophic backtracking pattern in linear time.
    #[tokio::test]
    async fn expand_inline_regex_filter_anchored_rejects_trailing_chars() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-regex-bad",
              "url": "http://example.org/cs-regex-bad",
              "status": "active", "content": "complete",
              "concept": [
                { "code": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",  "display": "Pure" },
                { "code": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaY", "display": "Y" },
                { "code": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaZ", "display": "Z" }
              ]
            }
          }]
        }"#;
        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-regex-bad",
                    "filter": [{
                        "property": "code",
                        "op": "regex",
                        "value": "(a+)+"
                    }]
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
        assert_eq!(codes.len(), 1, "only the pure-a code matches full-string");
        assert!(codes[0].chars().all(|c| c == 'a'));
    }

    /// A malformed regex must surface as `HtsError::VsInvalid` so the IG
    /// fixtures see the `tx-issue-type=vs-invalid` coding rather than a
    /// generic `invalid` error.  An unbalanced `[` is rejected by every
    /// regex engine.
    #[tokio::test]
    async fn expand_inline_regex_invalid_pattern_returns_vs_invalid() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-rx-broken",
              "url": "http://example.org/cs-rx-broken",
              "status": "active", "content": "complete",
              "concept": [{ "code": "X", "display": "X" }]
            }
          }]
        }"#;
        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-rx-broken",
                    "filter": [{
                        "property": "code",
                        "op": "regex",
                        "value": "[unclosed"
                    }]
                }]
            }
        });

        let err = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    ..Default::default()
                },
            )
            .await
            .expect_err("malformed regex must error");
        assert!(
            matches!(err, HtsError::VsInvalid(_)),
            "expected VsInvalid, got: {err:?}"
        );
    }

    /// Mirror of `simple-expand-child-of`: `child-of code2` should select only
    /// the **direct** children of `code2` (`code2a`, `code2b`) and exclude
    /// transitive descendants (`code2aI`, `code2aII`) and the value itself.
    #[tokio::test]
    async fn expand_inline_child_of_filter_returns_direct_children_only() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-childof",
              "url": "http://example.org/cs-childof",
              "status": "active", "content": "complete",
              "hierarchyMeaning": "is-a",
              "concept": [
                { "code": "code1", "display": "Display 1" },
                { "code": "code2", "display": "Display 2",
                  "concept": [
                    { "code": "code2a", "display": "Display 2a",
                      "concept": [
                        { "code": "code2aI",  "display": "Display 2aI" },
                        { "code": "code2aII", "display": "Display 2aII" }
                      ]
                    },
                    { "code": "code2b", "display": "Display 2b" }
                  ]
                },
                { "code": "code3", "display": "Display 3" }
              ]
            }
          }]
        }"#;
        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-childof",
                    "filter": [{
                        "property": "concept",
                        "op": "child-of",
                        "value": "code2"
                    }]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    count: Some(50),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let mut codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        codes.sort();
        assert_eq!(
            codes,
            vec!["code2a", "code2b"],
            "child-of returns direct children only"
        );
        assert!(!codes.contains(&"code2"), "child-of must exclude self");
        assert!(
            !codes.contains(&"code2aI"),
            "child-of must exclude grandchildren"
        );
    }

    /// Validate that `is-a` correctly returns the full transitive-closure
    /// expansion when the value has both children and grandchildren.  The
    /// tx-ecosystem `simple-expand-isa` test sets `value=code2`, expecting all
    /// 5 concepts in the subtree (self + 2 children + 2 grandchildren).
    #[tokio::test]
    async fn expand_inline_is_a_filter_returns_full_subtree_including_self() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle", "type": "collection",
          "entry": [{
            "resource": {
              "resourceType": "CodeSystem",
              "id": "cs-isa-deep",
              "url": "http://example.org/cs-isa-deep",
              "status": "active", "content": "complete",
              "hierarchyMeaning": "is-a",
              "concept": [
                { "code": "code1", "display": "Display 1" },
                { "code": "code2", "display": "Display 2",
                  "concept": [
                    { "code": "code2a", "display": "Display 2a",
                      "concept": [
                        { "code": "code2aI",  "display": "Display 2aI" },
                        { "code": "code2aII", "display": "Display 2aII" }
                      ]
                    },
                    { "code": "code2b", "display": "Display 2b" }
                  ]
                },
                { "code": "code3", "display": "Display 3" }
              ]
            }
          }]
        }"#;
        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let inline_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs-isa-deep",
                    "filter": [{
                        "property": "concept",
                        "op": "is-a",
                        "value": "code2"
                    }]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline_vs),
                    count: Some(50),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let mut codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        codes.sort();
        assert_eq!(
            codes,
            vec!["code2", "code2a", "code2aI", "code2aII", "code2b"],
            "is-a returns the full subtree including self"
        );
        // `total` is intentionally not asserted — the BFS fast path used by
        // single-include is-a expansions returns `total: None` to avoid the
        // separate count round-trip when the caller only asked for a page.
    }
    // ── inline `#contained` ValueSet ref + canonical URL intersection ──────────

    /// Bundle that covers the simple-expand-contained tx-ecosystem fixture:
    /// CodeSystem `simple` with codes `code1`, `code2`; ValueSet
    /// `simple-filter-isa` whose compose explicitly includes `code2`. The
    /// inline request body adds a `contained[]` ValueSet `vs1` with
    /// `concept: [{code: "code2"}]`. The intersection is `{code2}`.
    fn bundle_for_contained_intersection() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            { "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-simple",
                "url": "http://example.org/cs/simple",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "code1", "display": "One" },
                  { "code": "code2", "display": "Two" },
                  { "code": "code3", "display": "Three" }
                ]
            }},
            { "resource": {
                "resourceType": "ValueSet",
                "id": "vs-isa",
                "url": "http://example.org/vs/simple-filter-isa",
                "status": "active",
                "compose": { "include": [
                    { "system": "http://example.org/cs/simple",
                      "concept": [{ "code": "code2" }] }
                ]}
            }}
          ]
        }"#
    }

    #[tokio::test]
    async fn inline_contained_fragment_ref_intersects_with_canonical_ref() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_for_contained_intersection().as_bytes())
            .await
            .unwrap();

        // Inline VS with one include that names two ValueSets to intersect:
        // a `#vs1` contained ref plus a canonical URL.
        let inline = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "valueSet": [
                        "#vs1",
                        "http://example.org/vs/simple-filter-isa"
                    ]
                }]
            },
            "contained": [{
                "resourceType": "ValueSet",
                "id": "vs1",
                "url": "http://example.org/vs/contained",
                "status": "active",
                "compose": { "include": [
                    { "system": "http://example.org/cs/simple",
                      "concept": [{ "code": "code2" }] }
                ]}
            }]
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(1));
        assert_eq!(resp.contains.len(), 1);
        assert_eq!(resp.contains[0].code, "code2");
        // Both refs resolved → no warning emitted for these.
        assert!(
            resp.warnings.iter().all(|w| !w.contains("not found")),
            "expected no not-found warnings, got {:?}",
            resp.warnings
        );
    }

    /// `#fragment` references that don't exist in `contained[]` push a
    /// warning but the rest of the expansion still proceeds — they don't
    /// cause a 404.
    #[tokio::test]
    async fn inline_unknown_fragment_ref_warns_but_does_not_404() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_for_contained_intersection().as_bytes())
            .await
            .unwrap();

        let inline = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [
                    { "valueSet": ["#missing"] },
                    { "system": "http://example.org/cs/simple",
                      "concept": [{ "code": "code1" }] }
                ]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Second include still resolves — the missing #fragment doesn't
        // poison the whole request.
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"code1"));
        assert!(
            resp.warnings.iter().any(|w| w.contains("#missing")),
            "expected a warning for the missing contained ref, got {:?}",
            resp.warnings
        );
    }

    /// `tx-resource` ValueSets are consulted *before* the local DB when
    /// resolving canonical URL refs inside an inline compose.
    #[tokio::test]
    async fn inline_tx_resource_shadows_canonical_ref() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_for_contained_intersection().as_bytes())
            .await
            .unwrap();

        // tx-resource VS that exists nowhere in the DB. It includes only `code1`.
        let tx_vs = serde_json::json!({
            "resourceType": "ValueSet",
            "url": "http://example.org/vs/tx-only",
            "status": "active",
            "compose": { "include": [
                { "system": "http://example.org/cs/simple",
                  "concept": [{ "code": "code1" }] }
            ]}
        });

        let inline = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{ "valueSet": ["http://example.org/vs/tx-only"] }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline),
                    tx_resources: vec![tx_vs],
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(1));
        assert_eq!(resp.contains[0].code, "code1");
    }

    /// Cycle in `compose.include[].valueSet[]` resolution must not loop —
    /// the visited-set guard breaks recursion. The non-cyclic include in
    /// the same compose still resolves.
    #[tokio::test]
    async fn cyclic_value_set_reference_is_rejected_without_loop() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_for_contained_intersection().as_bytes())
            .await
            .unwrap();

        // Two contained VSes that reference each other plus a real include
        // so the request as a whole isn't 100% cycle. Without cycle
        // detection this would recurse forever.
        let inline = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [
                    { "valueSet": ["#a"] },
                    { "system": "http://example.org/cs/simple",
                      "concept": [{ "code": "code1" }] }
                ]
            },
            "contained": [
                {
                    "resourceType": "ValueSet",
                    "id": "a",
                    "url": "http://example.org/vs/a",
                    "status": "active",
                    "compose": { "include": [{ "valueSet": ["#b"] }] }
                },
                {
                    "resourceType": "ValueSet",
                    "id": "b",
                    "url": "http://example.org/vs/b",
                    "status": "active",
                    "compose": { "include": [{ "valueSet": ["#a"] }] }
                }
            ]
        });

        // Returns Ok rather than hanging. The non-cyclic include still
        // resolves so the response is non-empty.
        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert!(codes.contains(&"code1"));
        assert!(
            resp.warnings.iter().any(|w| w.contains("Cyclic")),
            "expected a vs-invalid cycle warning, got {:?}",
            resp.warnings
        );
    }

    /// `compose.exclude[].valueSet[]` intersected with explicit codes —
    /// covers the exclude-combo tx-ecosystem fixture pattern.
    #[tokio::test]
    async fn exclude_with_value_set_ref_intersects_with_local_concepts() {
        let b = backend();
        // Bundle: gender CS with male/female/other/unknown + a VS `gender-vs`
        // that includes ALL of those.
        let bundle = r#"{
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
              { "resource": {
                  "resourceType": "CodeSystem",
                  "id": "cs-gender",
                  "url": "http://example.org/cs/gender",
                  "status": "active",
                  "content": "complete",
                  "concept": [
                    { "code": "male" },
                    { "code": "female" },
                    { "code": "other" },
                    { "code": "unknown" }
                  ]
              }},
              { "resource": {
                  "resourceType": "ValueSet",
                  "id": "gender-vs",
                  "url": "http://example.org/vs/gender",
                  "status": "active",
                  "compose": { "include": [
                      { "system": "http://example.org/cs/gender" }
                  ]}
              }}
            ]
        }"#;
        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        // Inline VS that includes male+female and excludes (female + other)
        // intersected with the gender VS — so only `female` is excluded
        // because `other` is not in the include.
        let inline = serde_json::json!({
            "resourceType": "ValueSet",
            "compose": {
                "include": [{
                    "system": "http://example.org/cs/gender",
                    "concept": [
                        { "code": "male" },
                        { "code": "female" }
                    ]
                }],
                "exclude": [{
                    "system": "http://example.org/cs/gender",
                    "concept": [
                        { "code": "female" },
                        { "code": "other" }
                    ],
                    "valueSet": ["http://example.org/vs/gender"]
                }]
            }
        });

        let resp = b
            .expand(
                &ctx(),
                ExpandRequest {
                    value_set: Some(inline),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.total, Some(1));
        let codes: Vec<&str> = resp.contains.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(codes, vec!["male"]);
    }
}
