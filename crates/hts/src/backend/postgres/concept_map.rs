//! PostgreSQL implementation of [`ConceptMapOperations`].

#![cfg(feature = "postgres")]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use serde_json::json;
use std::collections::HashMap;

use crate::error::HtsError;
use crate::traits::ConceptMapOperations;
use crate::types::{
    ClosureRequest, ClosureResponse, ResourceSearchQuery, TranslateRequest, TranslateResponse,
    TranslationMatch,
};

use super::PostgresTerminologyBackend;
use super::code_system::build_synthetic_resource;

#[async_trait]
impl ConceptMapOperations for PostgresTerminologyBackend {
    async fn translate(
        &self,
        _ctx: &TenantContext,
        req: TranslateRequest,
    ) -> Result<TranslateResponse, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let rows = query_translate_elements(
            &client,
            &req.code,
            req.system.as_deref(),
            req.url.as_deref(),
            req.reverse,
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
            })
            .collect();

        let result = !matches.is_empty();

        Ok(TranslateResponse {
            result,
            message: if result {
                None
            } else {
                Some("No mapping found for the provided code".into())
            },
            matches,
        })
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
                .query("SELECT id FROM code_systems WHERE url = $1", &[system_url])
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

            let resource = resource_json.unwrap_or_else(|| {
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
    display: Option<String>,
}

/// Query `concept_map_elements` for matching translations.
///
/// `reverse = false` (default): search source_code, return target.
/// `reverse = true`: search target_code, return source.
async fn query_translate_elements(
    client: &tokio_postgres::Client,
    code: &str,
    system: Option<&str>,
    map_url: Option<&str>,
    reverse: bool,
    date: Option<&str>,
) -> Result<Vec<TranslateRow>, HtsError> {
    let sql = if !reverse {
        "SELECT cme.target_system, cme.target_code, cme.equivalence, cm.url, c.display
         FROM concept_map_elements cme
         JOIN concept_maps cm ON cm.id = cme.map_id
         LEFT JOIN code_systems cs_disp ON cs_disp.url = cme.target_system
         LEFT JOIN concepts c ON c.system_id = cs_disp.id AND c.code = cme.target_code
         WHERE cme.source_code = $1
           AND ($2::text IS NULL OR cme.source_system = $2)
           AND ($3::text IS NULL OR cm.url = $3)
           AND ($4::text IS NULL OR (cm.resource_json->>'date') <= $4)"
    } else {
        "SELECT cme.source_system, cme.source_code, cme.equivalence, cm.url, c.display
         FROM concept_map_elements cme
         JOIN concept_maps cm ON cm.id = cme.map_id
         LEFT JOIN code_systems cs_disp ON cs_disp.url = cme.source_system
         LEFT JOIN concepts c ON c.system_id = cs_disp.id AND c.code = cme.source_code
         WHERE cme.target_code = $1
           AND ($2::text IS NULL OR cme.target_system = $2)
           AND ($3::text IS NULL OR cm.url = $3)
           AND ($4::text IS NULL OR (cm.resource_json->>'date') <= $4)"
    };

    let rows = client
        .query(sql, &[&code, &system, &map_url, &date])
        .await
        .map_err(|e| HtsError::StorageError(format!("Query error: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|row| TranslateRow {
            concept_system: row.get(0),
            concept_code: row.get(1),
            equivalence: row.get(2),
            map_url: row.get(3),
            display: row.get(4),
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
