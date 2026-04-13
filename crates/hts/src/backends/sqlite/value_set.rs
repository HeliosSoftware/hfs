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
use std::collections::{HashMap, HashSet};

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

            // Resolve expansion codes — either from an explicit ValueSet or from an
            // implicit one defined by `CodeSystem.valueSet`.
            let all_codes = match resolve_value_set(&conn, &url, req.date.as_deref()) {
                Ok((vs_id, compose_json)) => {
                    // Normal path: try the expansion cache first.
                    let cached = fetch_cache(&conn, &vs_id)?;
                    if cached.is_empty() {
                        let codes = compute_expansion(&conn, compose_json.as_deref())?;
                        if let Some(limit) = req.max_expansion_size {
                            if codes.len() as u64 > u64::from(limit) {
                                return Err(HtsError::TooCostly(format!(
                                    "ValueSet expansion contains {} codes which exceeds \
                                     the server limit of {} (set HTS_MAX_EXPANSION_SIZE to raise it)",
                                    codes.len(),
                                    limit
                                )));
                            }
                        }
                        populate_cache(&conn, &vs_id, &codes)?;
                        codes
                    } else {
                        cached
                    }
                }
                Err(HtsError::NotFound(_)) => {
                    // Implicit ValueSet fallback: look for a CodeSystem whose
                    // `$.valueSet` property equals the requested URL.
                    let cs_url =
                        find_cs_for_implicit_vs(&conn, &url, req.date.as_deref())?;
                    let compose = serde_json::json!({
                        "include": [{ "system": cs_url }]
                    })
                    .to_string();
                    let codes = compute_expansion(&conn, Some(&compose))?;
                    if let Some(limit) = req.max_expansion_size {
                        if codes.len() as u64 > u64::from(limit) {
                            return Err(HtsError::TooCostly(format!(
                                "Implicit ValueSet expansion contains {} codes which exceeds \
                                 the server limit of {} (set HTS_MAX_EXPANSION_SIZE to raise it)",
                                codes.len(),
                                limit
                            )));
                        }
                    }
                    // No caching for implicit ValueSets (no row in value_sets table).
                    codes
                }
                Err(e) => return Err(e),
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
                });
            }

            let total = filtered.len() as u32;
            let offset = req.offset.unwrap_or(0) as usize;
            let count = req.count.map(|c| c as usize).unwrap_or(usize::MAX);

            let page: Vec<ExpansionContains> =
                filtered.into_iter().skip(offset).take(count).collect();

            Ok(ExpandResponse {
                total: Some(total),
                offset: req.offset,
                contains: page,
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

            // Unknown value set → false (not an error).
            let (vs_id, compose_json) = match resolve_value_set(&conn, &url, req.date.as_deref()) {
                Ok(vs) => vs,
                Err(HtsError::NotFound(_)) => {
                    return Ok(ValidateCodeResponse {
                        result: false,
                        message: Some(format!("Unknown value set: {url}")),
                        display: None,
                    });
                }
                Err(e) => return Err(e),
            };

            // Get or compute expansion.
            let cached = fetch_cache(&conn, &vs_id)?;
            let all_codes = if cached.is_empty() {
                let codes = compute_expansion(&conn, compose_json.as_deref())?;
                populate_cache(&conn, &vs_id, &codes)?;
                codes
            } else {
                cached
            };

            // Search the expansion for the requested code.
            // When `system` is provided, match on both system + code.
            // When `system` is absent, match on code alone (first hit).
            let found = if let Some(system) = req.system.as_deref() {
                all_codes
                    .iter()
                    .find(|c| c.system == system && c.code == req.code)
            } else {
                all_codes.iter().find(|c| c.code == req.code)
            };

            match found {
                None => Ok(ValidateCodeResponse {
                    result: false,
                    message: Some(format!(
                        "Code '{}' is not in value set '{url}'",
                        req.code
                    )),
                    display: None,
                }),
                Some(concept) => {
                    let mut message = None;
                    if let Some(expected) = req.display.as_deref() {
                        if let Some(actual) = concept.display.as_deref() {
                            if !actual.eq_ignore_ascii_case(expected) {
                                message = Some(format!(
                                    "Provided display '{expected}' does not match stored display '{actual}'"
                                ));
                            }
                        }
                    }
                    // Per FHIR spec, a display mismatch causes result=false (with a message).
                    Ok(ValidateCodeResponse {
                        result: message.is_none(),
                        message,
                        display: concept.display.clone(),
                    })
                }
            }
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
                .prepare(
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
        .prepare(
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
                tracing::warn!(
                    system_url,
                    "Skipping unknown code system in ValueSet compose"
                );
                continue;
            }
        };

        // Check for ECL / is-a filters before falling through to the explicit
        // code list or "all concepts" paths.
        if let Some(filter_result) = apply_compose_filters(conn, system_url, &system_id, inc)? {
            included.extend(filter_result);
        } else if let Some(explicit_codes) = inc["concept"].as_array() {
            // Explicit code list: fetch display for each listed code.
            for entry in explicit_codes {
                let code = match entry["code"].as_str() {
                    Some(c) => c.to_owned(),
                    None => continue,
                };

                let display: Option<String> = conn
                    .query_row(
                        "SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2",
                        rusqlite::params![system_id, code],
                        |row| row.get(0),
                    )
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
                .prepare("SELECT code, display FROM concepts WHERE system_id = ?1 ORDER BY code")
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
/// | `property`   | `op`   | Interpretation |
/// |--------------|--------|----------------|
/// | `constraint` | `=`    | Full ECL expression (e.g. `<< 404684003 \|Finding\|`) |
/// | `concept`    | `is-a` | Subsumption shorthand — translated to `<< <value>` |
///
/// Unrecognised `(property, op)` pairs emit a `WARN` trace event and are
/// treated as yielding an empty set so they do not silently expand the whole
/// code system.
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

    // `result` starts as `None` (no filters processed yet).  After the first
    // recognised filter it becomes `Some(set)`.  Subsequent recognised filters
    // are intersected into that set.  Unrecognised filters shrink the set to
    // empty (rather than being ignored) so they cannot expand it.
    let mut result: Option<Vec<ExpansionContains>> = None;
    let mut any_filter_seen = false;

    for f in filters {
        let property = f["property"].as_str().unwrap_or("");
        let op = f["op"].as_str().unwrap_or("");
        let value = f["value"].as_str().unwrap_or("");

        let ecl_expr: String = match (property, op) {
            ("constraint", "=") => value.to_owned(),
            ("concept", "is-a") => format!("<< {value}"),
            _ => {
                tracing::warn!(
                    property,
                    op,
                    "Unsupported compose filter — treating as empty set"
                );
                // Mark that we saw a filter so we don't fall through to
                // all-concepts, then intersect with empty to zero out any
                // previously accumulated set.
                any_filter_seen = true;
                result = Some(vec![]);
                continue;
            }
        };

        any_filter_seen = true;
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
            // Intersect with the running result (AND semantics).
            Some(prev) => {
                let keep: HashSet<String> = concepts.iter().map(|c| c.code.clone()).collect();
                prev.retain(|c| keep.contains(&c.code));
            }
            None => result = Some(concepts),
        }
    }

    // If we processed at least one filter entry (even if all were unrecognised)
    // return Some(result) so the caller does not fall back to all-concepts.
    // If result is still None at this point it means every filter was
    // unrecognised → return an empty expansion.
    if any_filter_seen && result.is_none() {
        return Ok(Some(vec![]));
    }

    Ok(result)
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
            .prepare(
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
fn populate_cache(
    conn: &Connection,
    vs_id: &str,
    codes: &[ExpansionContains],
) -> Result<(), HtsError> {
    conn.execute(
        "DELETE FROM value_set_expansions WHERE value_set_id = ?1",
        [vs_id],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    for item in codes {
        conn.execute(
            "INSERT OR IGNORE INTO value_set_expansions
             (value_set_id, system_url, code, display)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![vs_id, item.system, item.code, item.display],
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    }

    Ok(())
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

    // ── $validate-code: unknown value set returns false ────────────────────────

    #[tokio::test]
    async fn validate_code_unknown_value_set_returns_false() {
        let b = backend();
        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://unknown.org/vs".into()),
                    code: "A".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(!resp.result);
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
}
