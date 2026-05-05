//! SQLite implementation of [`ConceptMapOperations`].
//!
//! **`$translate`** queries `concept_map_elements` (and optionally filters by
//! `concept_maps.url` and/or source/target system). Supports reverse mode
//! (`reverse = true`) which swaps source↔target lookup direction.
//!
//! **`$closure`** takes a set of `CodingConcept` inputs, groups them by code
//! system, and uses a recursive CTE over `concept_hierarchy` to find all
//! ancestor-descendant pairs within the input set. Returns a FHIR ConceptMap
//! (as `serde_json::Value`) representing those subsumption relationships.

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use rusqlite::{Connection, OptionalExtension};
use serde_json::json;
use std::collections::HashMap;

use crate::error::HtsError;
use crate::traits::ConceptMapOperations;
use crate::types::{
    ClosureRequest, ClosureResponse, ResourceSearchQuery, TranslateRequest, TranslateResponse,
    TranslationMatch,
};

use super::SqliteTerminologyBackend;

// ── Trait impl ─────────────────────────────────────────────────────────────────

#[async_trait]
impl ConceptMapOperations for SqliteTerminologyBackend {
    async fn translate(
        &self,
        _ctx: &TenantContext,
        req: TranslateRequest,
    ) -> Result<TranslateResponse, HtsError> {
        let pool = self.pool().clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
            translate_sync(&conn, &req)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    async fn closure(
        &self,
        _ctx: &TenantContext,
        req: ClosureRequest,
    ) -> Result<ClosureResponse, HtsError> {
        let pool = self.pool().clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
            closure_sync(&conn, &req)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    /// Search ConceptMap resources by query parameters.
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
                     FROM concept_maps
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
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }
}

// ── $translate ─────────────────────────────────────────────────────────────────

fn translate_sync(
    conn: &Connection,
    req: &TranslateRequest,
) -> Result<TranslateResponse, HtsError> {
    // When a specific ConceptMap URL is requested, verify it exists first.
    // A missing URL → 404 (not supported / not loaded), distinct from "no match found".
    // Using EXISTS short-circuits on the first matching row (faster than COUNT(*)).
    if let Some(url) = req.url.as_deref() {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM concept_maps WHERE url = ?1)",
                [url],
                |row| row.get(0),
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        if !exists {
            return Err(HtsError::NotFound(format!("ConceptMap not found: {url}")));
        }
    }

    // Reverse mode is set explicitly via `reverse=true`, *or* implicitly when
    // the caller supplied `targetCode` instead of `sourceCode` (R5 form).
    let reverse = req.reverse || req.target_code.is_some();

    // Pick the lookup code: in reverse-via-targetCode mode use `target_code`;
    // otherwise use `code`.
    let lookup_code: &str = if req.target_code.is_some() {
        req.target_code.as_deref().unwrap_or("")
    } else {
        req.code.as_str()
    };

    // Pick the system that goes with the lookup code, and the "other side"
    // restriction that applies to the result.
    //   - Forward (default): search `req.system` (sourceSystem); restrict
    //     result side to `req.target_system`.
    //   - Reverse via `reverse=true`: callers historically pass the
    //     target-side system in `req.system`; restrict result (source side)
    //     to `req.target_system`.
    //   - Reverse via `targetCode` (R5): search `req.target_system`;
    //     restrict result (source side) to `req.system` (sourceSystem of
    //     the request).
    let (search_sys, other_side_sys) = if req.target_code.is_some() {
        (req.target_system.as_deref(), req.system.as_deref())
    } else {
        (req.system.as_deref(), req.target_system.as_deref())
    };

    let mut rows = query_translate_elements(
        conn,
        lookup_code,
        search_sys,
        other_side_sys,
        req.url.as_deref(),
        reverse,
        req.date.as_deref(),
    )?;

    // Some terminology packages encode multiple source codes as a comma-separated
    // string (e.g. "unconfirmed, provisional"). After import those are stored as
    // individual rows, but clients may still send the original compound string.
    // When no exact match is found and the code contains a comma, try each
    // individual token so compound-code queries still resolve.
    if rows.is_empty() && lookup_code.contains(',') {
        for token in lookup_code.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let token_rows = query_translate_elements(
                conn,
                token,
                search_sys,
                other_side_sys,
                req.url.as_deref(),
                reverse,
                req.date.as_deref(),
            )?;
            if !token_rows.is_empty() {
                rows = token_rows;
                break;
            }
        }
    }

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

/// Internal row type for translate queries.
struct TranslateRow {
    concept_system: String,
    concept_code: String,
    equivalence: String,
    map_url: String,
    map_version: Option<String>,
    display: Option<String>,
    /// The "input side" system (i.e. the side the request matched against).
    /// Used to populate the `source` Coding part in reverse responses.
    input_system: Option<String>,
    input_code: Option<String>,
}

/// Query `concept_map_elements` for matching translations.
///
/// Uses NULL-safe parameter binding so a single prepared statement handles
/// all combinations of optional system / target-system / map_url filters.
///
/// `reverse = false` (default): search source_code, return target.
/// `reverse = true`: search target_code, return source.
///
/// `other_side_sys` restricts the *result* side: in forward mode it filters
/// `target_system`; in reverse mode it filters `source_system`. This lets
/// callers say "translate code-X from source-CS into target-CS" without
/// pulling in unrelated maps that happen to mention the source code.
fn query_translate_elements(
    conn: &Connection,
    code: &str,
    system: Option<&str>,
    other_side_sys: Option<&str>,
    map_url: Option<&str>,
    reverse: bool,
    date: Option<&str>,
) -> Result<Vec<TranslateRow>, HtsError> {
    // Column names depend on direction.
    let (
        search_code_col,
        search_sys_col,
        other_sys_col,
        disp_sys_col,
        disp_code_col,
        res_sys_col,
        res_code_col,
    ) = if !reverse {
        (
            "cme.source_code",
            "cme.source_system",
            "cme.target_system",
            "cme.target_system",
            "cme.target_code",
            "cme.target_system",
            "cme.target_code",
        )
    } else {
        (
            "cme.target_code",
            "cme.target_system",
            "cme.source_system",
            "cme.source_system",
            "cme.source_code",
            "cme.source_system",
            "cme.source_code",
        )
    };

    // Inline the column names into the query string (no user input touches this).
    let sql = format!(
        "SELECT {res_sys_col}, {res_code_col}, cme.equivalence, cm.url, cm.version,
                c.display, {search_sys_col}, {search_code_col}
         FROM concept_map_elements cme
         JOIN concept_maps cm ON cm.id = cme.map_id
         LEFT JOIN code_systems cs_disp ON cs_disp.url = {disp_sys_col}
         LEFT JOIN concepts c ON c.system_id = cs_disp.id AND c.code = {disp_code_col}
         WHERE {search_code_col} = ?1
           AND (?2 IS NULL OR {search_sys_col} = ?2)
           AND (?3 IS NULL OR {other_sys_col} = ?3)
           AND (?4 IS NULL OR cm.url = ?4)
           AND (?5 IS NULL OR json_extract(cm.resource_json, '$.date') <= ?5)"
    );

    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|e| HtsError::StorageError(format!("Prepare error: {e}")))?;

    let rows = stmt
        .query_map(
            rusqlite::params![code, system, other_side_sys, map_url, date],
            |row| {
                Ok(TranslateRow {
                    concept_system: row.get(0)?,
                    concept_code: row.get(1)?,
                    equivalence: row.get(2)?,
                    map_url: row.get(3)?,
                    map_version: row.get(4)?,
                    display: row.get(5)?,
                    input_system: row.get(6)?,
                    input_code: row.get(7)?,
                })
            },
        )
        .map_err(|e| HtsError::StorageError(format!("Query error: {e}")))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| HtsError::StorageError(format!("Row error: {e}")))?;

    Ok(rows)
}

// ── $closure ───────────────────────────────────────────────────────────────────

fn closure_sync(conn: &Connection, req: &ClosureRequest) -> Result<ClosureResponse, HtsError> {
    // Empty input → empty closure.
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

    // Group codes by code system URL.
    let mut by_system: HashMap<String, Vec<String>> = HashMap::new();
    for concept in &req.concept {
        by_system
            .entry(concept.system.clone())
            .or_default()
            .push(concept.code.clone());
    }

    let mut groups: Vec<serde_json::Value> = Vec::new();

    for (system_url, codes) in &by_system {
        // Look up the internal code_system.id.
        let system_id_opt: Option<String> = conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = ?1",
                rusqlite::params![system_url],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HtsError::StorageError(format!("DB error: {e}")))?;

        let system_id = match system_id_opt {
            Some(id) => id,
            None => continue, // Unknown system; skip silently.
        };

        // For each code that has descendants in the input set, build an element.
        let mut elements: Vec<serde_json::Value> = Vec::new();

        for ancestor_code in codes {
            let mut targets: Vec<serde_json::Value> = Vec::new();

            for descendant_code in codes {
                if ancestor_code == descendant_code {
                    continue;
                }

                if is_ancestor_of(conn, &system_id, ancestor_code, descendant_code)? {
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

/// Returns `true` if `ancestor_code` is a (possibly transitive) ancestor of
/// `descendant_code` in the given code system.
///
/// Uses a recursive CTE that walks *up* from `descendant_code`, collecting
/// all ancestor codes, then checks whether `ancestor_code` appears in the set.
fn is_ancestor_of(
    conn: &Connection,
    system_id: &str,
    ancestor_code: &str,
    descendant_code: &str,
) -> Result<bool, HtsError> {
    let sql = "
        WITH RECURSIVE ancestors(code) AS (
            SELECT parent_code
            FROM concept_hierarchy
            WHERE system_id = ?1 AND child_code = ?2
            UNION
            SELECT ch.parent_code
            FROM concept_hierarchy ch
            INNER JOIN ancestors a ON ch.child_code = a.code
            WHERE ch.system_id = ?1
        )
        SELECT 1 FROM ancestors WHERE code = ?3
        LIMIT 1
    ";

    let found: Option<i32> = conn
        .query_row(
            sql,
            rusqlite::params![system_id, descendant_code, ancestor_code],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(format!("DB error: {e}")))?;

    Ok(found.is_some())
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::backends::sqlite::SqliteTerminologyBackend;
    use crate::traits::ConceptMapOperations;
    use crate::types::{ClosureRequest, CodingConcept, TranslateRequest};
    use helios_persistence::tenant::TenantContext;

    /// Seed helper: creates a code system, concepts, hierarchy, a concept map,
    /// and concept map elements for testing both $translate and $closure.
    fn seed_db(backend: &SqliteTerminologyBackend) {
        let conn = backend.pool().get().unwrap();
        conn.execute_batch(
            "-- Source code system
             INSERT INTO code_systems
                 (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('cs-src', 'http://example.org/src', '1.0', 'Source CS',
                     'active', 'complete', '2024-01-01', '2024-01-01');

             -- Target code system
             INSERT INTO code_systems
                 (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('cs-tgt', 'http://example.org/tgt', '1.0', 'Target CS',
                     'active', 'complete', '2024-01-01', '2024-01-01');

             -- Source concepts with hierarchy: A → B → C
             INSERT INTO concepts (id, system_id, code, display)
             VALUES (1, 'cs-src', 'A', 'Alpha'),
                    (2, 'cs-src', 'B', 'Beta'),
                    (3, 'cs-src', 'C', 'Gamma'),
                    (4, 'cs-src', 'D', 'Delta');

             INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
             VALUES ('cs-src', 'A', 'B'),
                    ('cs-src', 'B', 'C');

             -- Target concepts
             INSERT INTO concepts (id, system_id, code, display)
             VALUES (10, 'cs-tgt', 'X', 'X Display'),
                    (11, 'cs-tgt', 'Y', 'Y Display'),
                    (12, 'cs-tgt', 'Z', 'Z Display');

             -- ConceptMap: src → tgt
             INSERT INTO concept_maps
                 (id, url, version, source_uri, target_uri, status, created_at)
             VALUES ('cm1', 'http://example.org/cm', '1.0',
                     'http://example.org/src', 'http://example.org/tgt',
                     'active', '2024-01-01');

             -- Mappings: A→X (equivalent), B→Y (wider), B→Z (narrower)
             INSERT INTO concept_map_elements
                 (map_id, source_system, source_code, target_system, target_code, equivalence)
             VALUES ('cm1', 'http://example.org/src', 'A', 'http://example.org/tgt', 'X', 'equivalent'),
                    ('cm1', 'http://example.org/src', 'B', 'http://example.org/tgt', 'Y', 'wider'),
                    ('cm1', 'http://example.org/src', 'B', 'http://example.org/tgt', 'Z', 'narrower');",
        )
        .unwrap();
    }

    // ── $translate ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn translate_direct_mapping_returns_result() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);

        let ctx = TenantContext::system();
        let req = TranslateRequest {
            system: Some("http://example.org/src".into()),
            code: "A".into(),
            ..Default::default()
        };

        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(resp.result, "Expected result=true");
        assert_eq!(resp.matches.len(), 1);
        assert_eq!(resp.matches[0].concept_code, "X");
        assert_eq!(resp.matches[0].equivalence, "equivalent");
        assert_eq!(
            resp.matches[0].concept_display.as_deref(),
            Some("X Display")
        );
        assert_eq!(
            resp.matches[0].source.as_deref(),
            Some("http://example.org/cm")
        );
    }

    #[tokio::test]
    async fn translate_multiple_targets() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);

        let ctx = TenantContext::system();
        let req = TranslateRequest {
            system: Some("http://example.org/src".into()),
            code: "B".into(),
            ..Default::default()
        };

        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(resp.result);
        assert_eq!(resp.matches.len(), 2, "B maps to Y and Z");

        let codes: Vec<&str> = resp
            .matches
            .iter()
            .map(|m| m.concept_code.as_str())
            .collect();
        assert!(codes.contains(&"Y"));
        assert!(codes.contains(&"Z"));
    }

    #[tokio::test]
    async fn translate_no_match_returns_false() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);

        let ctx = TenantContext::system();
        let req = TranslateRequest {
            system: Some("http://example.org/src".into()),
            code: "D".into(), // D has no mapping
            ..Default::default()
        };

        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(!resp.result, "Expected result=false for unmapped code");
        assert!(resp.matches.is_empty());
        assert!(resp.message.is_some());
    }

    #[tokio::test]
    async fn translate_filtered_by_map_url() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);

        let ctx = TenantContext::system();
        // Filter by the known map URL — should still find A→X.
        let req = TranslateRequest {
            url: Some("http://example.org/cm".into()),
            system: Some("http://example.org/src".into()),
            code: "A".into(),
            ..Default::default()
        };
        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(resp.result);
        assert_eq!(resp.matches[0].concept_code, "X");

        // Filter by a non-existent map URL — should return NotFound.
        let req_bad = TranslateRequest {
            url: Some("http://unknown.org/cm".into()),
            system: Some("http://example.org/src".into()),
            code: "A".into(),
            ..Default::default()
        };
        let err = backend.translate(&ctx, req_bad).await.unwrap_err();
        assert!(matches!(err, crate::error::HtsError::NotFound(_)));
    }

    #[tokio::test]
    async fn translate_reverse_finds_source() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);

        let ctx = TenantContext::system();
        // Reverse: given target code X (in target system), find source code A.
        let req = TranslateRequest {
            system: Some("http://example.org/tgt".into()),
            code: "X".into(),
            reverse: true,
            ..Default::default()
        };

        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(resp.result);
        assert_eq!(resp.matches.len(), 1);
        assert_eq!(resp.matches[0].concept_code, "A");
        assert_eq!(resp.matches[0].concept_system, "http://example.org/src");
    }

    #[tokio::test]
    async fn translate_code_only_no_system_filter() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);

        let ctx = TenantContext::system();
        // No system filter — still finds mapping by code alone.
        let req = TranslateRequest {
            code: "A".into(),
            ..Default::default()
        };

        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(resp.result);
        assert_eq!(resp.matches[0].concept_code, "X");
    }

    /// `target_system` filter restricts the result side, not just the request.
    /// Without a candidate map matching the requested target system, no
    /// matches should be returned.
    #[tokio::test]
    async fn translate_filtered_by_target_system_no_match() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);

        let ctx = TenantContext::system();
        let req = TranslateRequest {
            system: Some("http://example.org/src".into()),
            target_system: Some("http://nope.example.org/cs".into()),
            code: "A".into(),
            ..Default::default()
        };

        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(!resp.result);
    }

    /// Reverse via R5 `target_code` returns the source-side Coding fields and
    /// uses `target_system` as the lookup-side restriction.
    #[tokio::test]
    async fn translate_reverse_via_target_code_returns_source_fields() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);

        let ctx = TenantContext::system();
        let req = TranslateRequest {
            // R5-style: sourceSystem narrows source side; targetCode/system
            // identify the code being reverse-translated.
            system: Some("http://example.org/src".into()),
            target_system: Some("http://example.org/tgt".into()),
            target_code: Some("X".into()),
            // `code` left empty intentionally (driven by `target_code`).
            code: String::new(),
            ..Default::default()
        };

        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(resp.result);
        assert_eq!(resp.matches.len(), 1);
        // Result `concept` is the source side (A in src CS).
        assert_eq!(resp.matches[0].concept_code, "A");
        assert_eq!(resp.matches[0].concept_system, "http://example.org/src");
        // The input-side fields carry the original target Coding (X in tgt CS).
        assert_eq!(resp.matches[0].source_code.as_deref(), Some("X"));
        assert_eq!(
            resp.matches[0].source_system.as_deref(),
            Some("http://example.org/tgt")
        );
        // Map version is included so handlers can build originMap canonicals.
        assert_eq!(resp.matches[0].map_version.as_deref(), Some("1.0"));
    }

    // ── $closure ───────────────────────────────────────────────────────────────

    fn coding(system: &str, code: &str) -> CodingConcept {
        CodingConcept {
            system: system.into(),
            code: code.into(),
            display: None,
        }
    }

    #[tokio::test]
    async fn closure_empty_concepts_returns_empty_map() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        let req = ClosureRequest {
            name: "my-closure".into(),
            concept: vec![],
            version: None,
        };

        let resp = backend.closure(&ctx, req).await.unwrap();
        assert_eq!(resp.name, "my-closure");
        let cm = resp.concept_map.unwrap();
        assert_eq!(cm["group"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn closure_three_level_hierarchy() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);
        let ctx = TenantContext::system();

        // A→B→C: closure of {A, B, C} should include (A,B), (A,C), (B,C)
        let req = ClosureRequest {
            name: "test".into(),
            concept: vec![
                coding("http://example.org/src", "A"),
                coding("http://example.org/src", "B"),
                coding("http://example.org/src", "C"),
            ],
            version: None,
        };

        let resp = backend.closure(&ctx, req).await.unwrap();
        let cm = resp.concept_map.unwrap();
        let groups = cm["group"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "All codes are in the same system");

        let elements = groups[0]["element"].as_array().unwrap();

        // Collect all (ancestor_code, descendant_code) pairs from the ConceptMap.
        let mut pairs: Vec<(String, String)> = Vec::new();
        for elem in elements {
            let ancestor = elem["code"].as_str().unwrap().to_string();
            for target in elem["target"].as_array().unwrap() {
                pairs.push((
                    ancestor.clone(),
                    target["code"].as_str().unwrap().to_string(),
                ));
            }
        }

        // A subsumes B and C; B subsumes C.
        assert!(pairs.contains(&("A".into(), "B".into())), "A subsumes B");
        assert!(
            pairs.contains(&("A".into(), "C".into())),
            "A subsumes C (transitive)"
        );
        assert!(pairs.contains(&("B".into(), "C".into())), "B subsumes C");
        // D is not in input, so no pair involving D.
        assert!(!pairs.iter().any(|(_, d)| d == "D"));
    }

    #[tokio::test]
    async fn closure_unrelated_codes_produce_no_pairs() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);
        let ctx = TenantContext::system();

        // A and D have no hierarchical relationship.
        let req = ClosureRequest {
            name: "test".into(),
            concept: vec![
                coding("http://example.org/src", "A"),
                coding("http://example.org/src", "D"),
            ],
            version: None,
        };

        let resp = backend.closure(&ctx, req).await.unwrap();
        let cm = resp.concept_map.unwrap();
        let groups = cm["group"].as_array().unwrap();
        assert_eq!(groups.len(), 0, "No pairs for unrelated codes");
    }

    #[tokio::test]
    async fn closure_unknown_system_skipped() {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        seed_db(&backend);
        let ctx = TenantContext::system();

        let req = ClosureRequest {
            name: "test".into(),
            concept: vec![
                coding("http://unknown.org/cs", "A"),
                coding("http://unknown.org/cs", "B"),
            ],
            version: None,
        };

        let resp = backend.closure(&ctx, req).await.unwrap();
        let cm = resp.concept_map.unwrap();
        // Unknown system is silently skipped — no groups in output.
        assert_eq!(cm["group"].as_array().unwrap().len(), 0);
    }

    // ── Integration: import Bundle → $translate end-to-end ────────────────────

    #[tokio::test]
    async fn translate_after_bundle_import() {
        use crate::import::BundleImportBackend;

        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let ctx = TenantContext::system();

        // Bundle with source CS, target CS, and a ConceptMap linking them.
        let bundle = serde_json::json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "CodeSystem",
                        "id": "cs-alpha",
                        "url": "http://bundle-test.org/src",
                        "name": "SourceCS",
                        "status": "active",
                        "content": "complete",
                        "concept": [
                            {"code": "100", "display": "Hundred"}
                        ]
                    }
                },
                {
                    "resource": {
                        "resourceType": "CodeSystem",
                        "id": "cs-beta",
                        "url": "http://bundle-test.org/tgt",
                        "name": "TargetCS",
                        "status": "active",
                        "content": "complete",
                        "concept": [
                            {"code": "200", "display": "Two Hundred"}
                        ]
                    }
                },
                {
                    "resource": {
                        "resourceType": "ConceptMap",
                        "id": "cm-bundle",
                        "url": "http://bundle-test.org/cm",
                        "name": "TestCM",
                        "status": "active",
                        "group": [
                            {
                                "source": "http://bundle-test.org/src",
                                "target": "http://bundle-test.org/tgt",
                                "element": [
                                    {
                                        "code": "100",
                                        "target": [
                                            {
                                                "code": "200",
                                                "equivalence": "equivalent"
                                            }
                                        ]
                                    }
                                ]
                            }
                        ]
                    }
                }
            ]
        });

        let bundle_bytes = serde_json::to_vec(&bundle).unwrap();
        let stats = backend.import_bundle(&ctx, &bundle_bytes).await.unwrap();
        assert_eq!(stats.code_systems, 2);
        assert_eq!(stats.concept_maps, 1);

        // Now translate 100 → 200.
        let req = TranslateRequest {
            system: Some("http://bundle-test.org/src".into()),
            code: "100".into(),
            ..Default::default()
        };

        let resp = backend.translate(&ctx, req).await.unwrap();
        assert!(resp.result);
        assert_eq!(resp.matches.len(), 1);
        assert_eq!(resp.matches[0].concept_code, "200");
        assert_eq!(resp.matches[0].equivalence, "equivalent");
    }
}
