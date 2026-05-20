//! PostgreSQL implementation of [`ConceptMapOperations`].

#![cfg(feature = "postgres")]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::HtsError;
use crate::traits::ConceptMapOperations;
use crate::types::{
    ClosureRequest, ClosureResponse, ResourceSearchQuery, TranslateRequest, TranslateResponse,
    TranslationMatch,
};

use super::{PG_TRANSLATE_RESPONSE_CACHE_MAX, PostgresTerminologyBackend};
use super::code_system::build_synthetic_resource;

/// Build the $translate response cache key. Folds every TranslateRequest
/// field that affects the output rows.
fn translate_cache_key(req: &TranslateRequest) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        req.url.as_deref().unwrap_or(""),
        req.system.as_deref().unwrap_or(""),
        req.code,
        req.source.as_deref().unwrap_or(""),
        req.target.as_deref().unwrap_or(""),
        req.target_system.as_deref().unwrap_or(""),
        req.target_code.as_deref().unwrap_or(""),
        if req.reverse { "1" } else { "0" },
        req.date.as_deref().unwrap_or(""),
    )
}

#[async_trait]
impl ConceptMapOperations for PostgresTerminologyBackend {
    async fn translate(
        &self,
        _ctx: &TenantContext,
        req: TranslateRequest,
    ) -> Result<TranslateResponse, HtsError> {
        // CM01/CM02 hot-path memo — bench pool entries repeat the same
        // translation tuples across 50 VUs.
        let cache_key = translate_cache_key(&req);
        if let Ok(read) = self.translate_response_cache.read() {
            if let Some(arc) = read.get(&cache_key) {
                return Ok((**arc).clone());
            }
        }

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        // Reverse mode is set explicitly via `reverse=true`, *or* implicitly
        // when the caller supplied `targetCode` instead of `sourceCode` (R5).
        let reverse = req.reverse || req.target_code.is_some();
        let lookup_code: &str = if req.target_code.is_some() {
            req.target_code.as_deref().unwrap_or("")
        } else {
            req.code.as_str()
        };
        let (search_sys, other_side_sys) = if req.target_code.is_some() {
            (req.target_system.as_deref(), req.system.as_deref())
        } else {
            (req.system.as_deref(), req.target_system.as_deref())
        };

        let rows = query_translate_elements(
            &client,
            lookup_code,
            search_sys,
            other_side_sys,
            req.url.as_deref(),
            reverse,
            req.date.as_deref(),
        )
        .await?;

        let matches: Vec<TranslationMatch> = rows
            .into_iter()
            .map(|r| TranslationMatch {
                equivalence: r.equivalence,
                concept_system: r.concept_system,
                concept_code: r.concept_code,
                concept_display: r.display,
                source: Some(r.map_url),
                map_version: r.map_version,
                source_system: r.input_system,
                source_code: r.input_code,
            })
            .collect();

        let result = !matches.is_empty();

        let response = TranslateResponse {
            result,
            message: if result {
                None
            } else {
                Some("No mapping found for the provided code".into())
            },
            matches,
        };
        if let Ok(mut w) = self.translate_response_cache.write() {
            if w.len() < PG_TRANSLATE_RESPONSE_CACHE_MAX || w.contains_key(&cache_key) {
                w.insert(cache_key, Arc::new(response.clone()));
            }
        }
        Ok(response)
    }

    async fn closure(
        &self,
        _ctx: &TenantContext,
        req: ClosureRequest,
    ) -> Result<ClosureResponse, HtsError> {
        if req.concept.is_empty() {
            return Ok(ClosureResponse {
                name: req.name.clone(),
                version: req.version.clone(),
                concept_map: Some(json!({
                    "resourceType": "ConceptMap",
                    "name": req.name,
                    "status": "active",
                    "group": []
                })),
            });
        }

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let mut by_system: HashMap<String, Vec<String>> = HashMap::new();
        for concept in &req.concept {
            by_system
                .entry(concept.system.clone())
                .or_default()
                .push(concept.code.clone());
        }

        let mut groups: Vec<serde_json::Value> = Vec::new();

        for (system_url, codes) in &by_system {
            let id_rows = client
                .query(
                    "SELECT id FROM code_systems WHERE url = $1 \
                     ORDER BY COALESCE(version, '') DESC LIMIT 1",
                    &[system_url],
                )
                .await
                .map_err(|e| HtsError::StorageError(format!("DB error: {e}")))?;

            let system_id: String = match id_rows.into_iter().next() {
                Some(r) => r.get(0),
                None => continue,
            };

            let mut elements: Vec<serde_json::Value> = Vec::new();

            for ancestor_code in codes {
                let mut targets: Vec<serde_json::Value> = Vec::new();

                for descendant_code in codes {
                    if ancestor_code == descendant_code {
                        continue;
                    }

                    if is_ancestor_of(&client, &system_id, ancestor_code, descendant_code).await? {
                        targets.push(json!({
                            "code": descendant_code,
                            "equivalence": "subsumes"
                        }));
                    }
                }

                if !targets.is_empty() {
                    elements.push(json!({
                        "code": ancestor_code,
                        "target": targets
                    }));
                }
            }

            if !elements.is_empty() {
                groups.push(json!({
                    "source": system_url,
                    "target": system_url,
                    "element": elements
                }));
            }
        }

        let concept_map = json!({
            "resourceType": "ConceptMap",
            "name": req.name,
            "status": "active",
            "group": groups
        });

        Ok(ClosureResponse {
            name: req.name.clone(),
            version: req.version.clone(),
            concept_map: Some(concept_map),
        })
    }

    async fn search(
        &self,
        _ctx: &TenantContext,
        query: ResourceSearchQuery,
    ) -> Result<Vec<serde_json::Value>, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let limit = i64::from(query.count.unwrap_or(20));
        let offset = i64::from(query.offset.unwrap_or(0));

        let rows = client
            .query(
                "SELECT id, url, version, name, title, status, resource_json
                 FROM concept_maps
                 WHERE ($1::text IS NULL OR url = $1)
                   AND ($2::text IS NULL OR version = $2)
                   AND ($3::text IS NULL OR name = $3)
                   AND ($4::text IS NULL OR title = $4)
                   AND ($5::text IS NULL OR status = $5)
                 ORDER BY created_at
                 LIMIT $6 OFFSET $7",
                &[
                    &query.url,
                    &query.version,
                    &query.name,
                    &query.title,
                    &query.status,
                    &limit,
                    &offset,
                ],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let url: String = row.get(1);
            let version: Option<String> = row.get(2);
            let name: Option<String> = row.get(3);
            let title: Option<String> = row.get(4);
            let status: String = row.get(5);
            let resource_json: Option<serde_json::Value> = row.get(6);

            let mut resource = resource_json.unwrap_or_else(|| {
                build_synthetic_resource(
                    "ConceptMap",
                    &id,
                    &url,
                    version.as_deref(),
                    name.as_deref(),
                    title.as_deref(),
                    &status,
                )
            });
            // Ensure the resource id matches the table's authoritative id column
            // (may differ from resource_json after a URL-conflict upsert).
            if let Some(obj) = resource.as_object_mut() {
                obj.insert("id".to_string(), serde_json::Value::String(id));
            }
            results.push(resource);
        }
        Ok(results)
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Internal row type for translate queries.
struct TranslateRow {
    concept_system: String,
    concept_code: String,
    equivalence: String,
    map_url: String,
    map_version: Option<String>,
    display: Option<String>,
    input_system: Option<String>,
    input_code: Option<String>,
}

/// Query `concept_map_elements` for matching translations.
///
/// `reverse = false` (default): search source_code, return target.
/// `reverse = true`: search target_code, return source.
///
/// `other_side_sys` restricts the result-side system (target side in forward
/// mode, source side in reverse mode).
async fn query_translate_elements(
    client: &tokio_postgres::Client,
    code: &str,
    system: Option<&str>,
    other_side_sys: Option<&str>,
    map_url: Option<&str>,
    reverse: bool,
    date: Option<&str>,
) -> Result<Vec<TranslateRow>, HtsError> {
    // Per FHIR $translate semantics, the response's `concept` Coding is always
    // the ConceptMap element's target side, and `source` is always its source
    // side — INDEPENDENT of forward vs reverse. Only the LOOKUP column
    // changes between modes (source.code for forward, target.code for
    // reverse). The IG `translate/translate-reverse` fixture pins this:
    // input target/code1, output concept = target/code1 (the looked-up
    // target), source = source/code-1 (the resolved source). Mirrors
    // sqlite/concept_map.rs:283-338.
    //
    // Also: don't echo the resolved-side display. The IG fixtures expect
    // bare {system, code} Codings without `display`, and SQLite hard-codes
    // None for the same reason. The LEFT JOIN was a PG over-reach.
    let (lookup_code_col, lookup_sys_col, other_sys_col) = if !reverse {
        ("cme.source_code", "cme.source_system", "cme.target_system")
    } else {
        ("cme.target_code", "cme.target_system", "cme.source_system")
    };
    let sql = format!(
        "SELECT cme.target_system, cme.target_code, cme.equivalence, cm.url, cm.version,
                NULL::text AS display, cme.source_system, cme.source_code
         FROM concept_map_elements cme
         JOIN concept_maps cm ON cm.id = cme.map_id
         WHERE {lookup_code_col} = $1
           AND ($2::text IS NULL OR {lookup_sys_col} = $2)
           AND ($3::text IS NULL OR {other_sys_col} = $3)
           AND ($4::text IS NULL OR cm.url = $4)
           AND ($5::text IS NULL OR (cm.resource_json->>'date') <= $5)"
    );

    let rows = client
        .query(&sql, &[&code, &system, &other_side_sys, &map_url, &date])
        .await
        .map_err(|e| HtsError::StorageError(format!("Query error: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|row| TranslateRow {
            concept_system: row.get(0),
            concept_code: row.get(1),
            equivalence: row.get(2),
            map_url: row.get(3),
            map_version: row.get(4),
            display: row.get(5),
            input_system: row.get(6),
            input_code: row.get(7),
        })
        .collect())
}

/// Returns `true` if `ancestor_code` is a (possibly transitive) ancestor of
/// `descendant_code` in the given code system.
async fn is_ancestor_of(
    client: &tokio_postgres::Client,
    system_id: &str,
    ancestor_code: &str,
    descendant_code: &str,
) -> Result<bool, HtsError> {
    let rows = client
        .query(
            "WITH RECURSIVE ancestors(code) AS (
                SELECT parent_code
                FROM concept_hierarchy
                WHERE system_id = $1 AND child_code = $2
                UNION
                SELECT ch.parent_code
                FROM concept_hierarchy ch
                INNER JOIN ancestors a ON ch.child_code = a.code
                WHERE ch.system_id = $1
            )
            SELECT 1 FROM ancestors WHERE code = $3
            LIMIT 1",
            &[&system_id, &descendant_code, &ancestor_code],
        )
        .await
        .map_err(|e| HtsError::StorageError(format!("DB error: {e}")))?;

    Ok(!rows.is_empty())
}
