//! SQLite-backed ECL evaluator.
//!
//! Translates a parsed [`EclExpr`] into one or more SQL queries against the
//! `concepts` and `concept_hierarchy` tables and returns the matching set of
//! `(code, display)` pairs.
//!
//! # Strategy
//!
//! Each operator maps to a recursive CTE:
//!
//! | Operator | SQL pattern |
//! |----------|-------------|
//! | `< id` | Walk `concept_hierarchy` from `parent_code = id` downward |
//! | `<< id` | Same but seed with `id` itself |
//! | `> id` | Walk `concept_hierarchy` from `child_code = id` upward |
//! | `>> id` | Same but seed with `id` itself |
//! | `id` (exact) | Single row lookup in `concepts` |
//! | `*` | All rows in `concepts` for this system |
//!
//! `AND` / `OR` / `MINUS` are evaluated by collecting both sides into
//! `HashSet<String>` (code-keyed) and performing set operations in Rust.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension};

use crate::error::HtsError;

use super::parser::{ConceptOperator, EclExpr, FocusConcept};

/// A resolved concept from the database.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConcept {
    pub code: String,
    pub display: Option<String>,
}

/// Evaluate an [`EclExpr`] against the given code system and return the
/// matching concepts ordered by code.
///
/// `system_id` is the opaque `code_systems.id` value (UUID string) for the
/// target code system (typically SNOMED CT).
pub fn evaluate(
    conn: &Connection,
    system_id: &str,
    expr: &EclExpr,
) -> Result<Vec<ResolvedConcept>, HtsError> {
    let map = eval_to_map(conn, system_id, expr)?;
    let mut result: Vec<ResolvedConcept> = map.into_values().collect();
    result.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(result)
}

// ── Core recursive evaluator ──────────────────────────────────────────────────

/// Evaluate `expr` to a `HashMap<code, ResolvedConcept>` for set operations.
fn eval_to_map(
    conn: &Connection,
    system_id: &str,
    expr: &EclExpr,
) -> Result<HashMap<String, ResolvedConcept>, HtsError> {
    match expr {
        EclExpr::Focus { op, concept } => eval_focus(conn, system_id, op.as_ref(), concept),

        EclExpr::And(left, right) => {
            let l = eval_to_map(conn, system_id, left)?;
            let r = eval_to_map(conn, system_id, right)?;
            // Intersection: keep codes present in both sides.
            let result = l
                .into_iter()
                .filter(|(code, _)| r.contains_key(code.as_str()))
                .collect();
            Ok(result)
        }

        EclExpr::Or(left, right) => {
            let mut l = eval_to_map(conn, system_id, left)?;
            let r = eval_to_map(conn, system_id, right)?;
            // Union: right side fills gaps (left takes precedence for display).
            for (code, concept) in r {
                l.entry(code).or_insert(concept);
            }
            Ok(l)
        }

        EclExpr::Minus(left, right) => {
            let l = eval_to_map(conn, system_id, left)?;
            let r_keys: HashSet<String> =
                eval_to_map(conn, system_id, right)?.into_keys().collect();
            // Difference: remove codes present in the right side.
            let result = l
                .into_iter()
                .filter(|(code, _)| !r_keys.contains(code.as_str()))
                .collect();
            Ok(result)
        }
    }
}

/// Evaluate a focus concept with its optional unary operator.
fn eval_focus(
    conn: &Connection,
    system_id: &str,
    op: Option<&ConceptOperator>,
    concept: &FocusConcept,
) -> Result<HashMap<String, ResolvedConcept>, HtsError> {
    match (op, concept) {
        // Wildcard — return all concepts in the system.
        (_, FocusConcept::Wildcard) => fetch_all(conn, system_id),

        // No operator — exact concept lookup.
        (None, FocusConcept::Id(id)) => fetch_exact(conn, system_id, id),

        // < id — strict descendants.
        (Some(ConceptOperator::DescendantOf), FocusConcept::Id(id)) => {
            fetch_descendants(conn, system_id, id, false)
        }

        // << id — self + descendants.
        (Some(ConceptOperator::DescendantOrSelf), FocusConcept::Id(id)) => {
            fetch_descendants(conn, system_id, id, true)
        }

        // > id — strict ancestors.
        (Some(ConceptOperator::AncestorOf), FocusConcept::Id(id)) => {
            fetch_ancestors(conn, system_id, id, false)
        }

        // >> id — self + ancestors.
        (Some(ConceptOperator::AncestorOrSelf), FocusConcept::Id(id)) => {
            fetch_ancestors(conn, system_id, id, true)
        }
    }
}

// ── SQL helpers ───────────────────────────────────────────────────────────────

/// Fetch a single concept by code.
fn fetch_exact(
    conn: &Connection,
    system_id: &str,
    code: &str,
) -> Result<HashMap<String, ResolvedConcept>, HtsError> {
    let result: Option<Option<String>> = conn
        .query_row(
            "SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2",
            rusqlite::params![system_id, code],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let mut map = HashMap::new();
    if let Some(display) = result {
        map.insert(
            code.to_string(),
            ResolvedConcept {
                code: code.to_string(),
                display,
            },
        );
    }
    Ok(map)
}

/// Fetch all concepts in the system.
fn fetch_all(
    conn: &Connection,
    system_id: &str,
) -> Result<HashMap<String, ResolvedConcept>, HtsError> {
    let mut stmt = conn
        .prepare("SELECT code, display FROM concepts WHERE system_id = ?1")
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows = stmt
        .query_map(rusqlite::params![system_id], |row| {
            Ok(ResolvedConcept {
                code: row.get(0)?,
                display: row.get(1)?,
            })
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows.into_iter().map(|c| (c.code.clone(), c)).collect())
}

/// Fetch descendants of `root_code` using a recursive CTE.
///
/// When `include_self` is `true` (`<< operator`), the root concept is also
/// included in the result.
fn fetch_descendants(
    conn: &Connection,
    system_id: &str,
    root_code: &str,
    include_self: bool,
) -> Result<HashMap<String, ResolvedConcept>, HtsError> {
    // The CTE seeds with the immediate children of root_code (< operator).
    // For << (include_self), we additionally union in the root itself.
    let sql = "
        WITH RECURSIVE desc_cte(code) AS (
            SELECT child_code
            FROM   concept_hierarchy
            WHERE  system_id = ?1 AND parent_code = ?2
            UNION
            SELECT h.child_code
            FROM   concept_hierarchy h
            INNER JOIN desc_cte d ON h.parent_code = d.code
            WHERE  h.system_id = ?1
        )
        SELECT c.code, c.display
        FROM   concepts c
        INNER JOIN desc_cte d ON c.code = d.code
        WHERE  c.system_id = ?1
    ";

    let mut map = collect_query(conn, sql, system_id, root_code)?;

    if include_self {
        // Also add the root concept itself.
        let self_row: Option<Option<String>> = conn
            .query_row(
                "SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2",
                rusqlite::params![system_id, root_code],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e: rusqlite::Error| HtsError::StorageError(e.to_string()))?;

        if let Some(display) = self_row {
            map.entry(root_code.to_string()).or_insert(ResolvedConcept {
                code: root_code.to_string(),
                display,
            });
        }
    }

    Ok(map)
}

/// Fetch ancestors of `start_code` using a recursive CTE.
///
/// When `include_self` is `true` (`>> operator`), the start concept is also
/// included in the result.
fn fetch_ancestors(
    conn: &Connection,
    system_id: &str,
    start_code: &str,
    include_self: bool,
) -> Result<HashMap<String, ResolvedConcept>, HtsError> {
    let sql = "
        WITH RECURSIVE anc_cte(code) AS (
            SELECT parent_code
            FROM   concept_hierarchy
            WHERE  system_id = ?1 AND child_code = ?2
            UNION
            SELECT h.parent_code
            FROM   concept_hierarchy h
            INNER JOIN anc_cte a ON h.child_code = a.code
            WHERE  h.system_id = ?1
        )
        SELECT c.code, c.display
        FROM   concepts c
        INNER JOIN anc_cte a ON c.code = a.code
        WHERE  c.system_id = ?1
    ";

    let mut map = collect_query(conn, sql, system_id, start_code)?;

    if include_self {
        let self_row: Option<Option<String>> = conn
            .query_row(
                "SELECT display FROM concepts WHERE system_id = ?1 AND code = ?2",
                rusqlite::params![system_id, start_code],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e: rusqlite::Error| HtsError::StorageError(e.to_string()))?;

        if let Some(display) = self_row {
            map.entry(start_code.to_string())
                .or_insert(ResolvedConcept {
                    code: start_code.to_string(),
                    display,
                });
        }
    }

    Ok(map)
}

/// Execute a two-parameter `(system_id, code)` query and collect results into
/// a `HashMap<code, ResolvedConcept>`.
fn collect_query(
    conn: &Connection,
    sql: &str,
    system_id: &str,
    code: &str,
) -> Result<HashMap<String, ResolvedConcept>, HtsError> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows = stmt
        .query_map(rusqlite::params![system_id, code], |row| {
            Ok(ResolvedConcept {
                code: row.get(0)?,
                display: row.get(1)?,
            })
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows.into_iter().map(|c| (c.code.clone(), c)).collect())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backends::SqliteTerminologyBackend;
    use crate::ecl::parser;

    /// Minimal hierarchy for tests:
    ///
    /// ```text
    /// ROOT (000)
    /// └── Disease (100)
    ///     ├── Infectious (101)
    ///     │   ├── Bacterial (102)
    ///     │   └── Viral (103)
    ///     └── Chronic (104)
    ///         └── Diabetes (105)
    /// ```
    fn setup_db() -> (SqliteTerminologyBackend, String) {
        let backend = SqliteTerminologyBackend::in_memory().unwrap();
        let pool = backend.pool();
        let conn = pool.get().unwrap();

        let now = "2024-01-01T00:00:00Z";
        let sys_id = "test-system-id";

        conn.execute(
            "INSERT INTO code_systems (id, url, version, name, status, content, created_at, updated_at)
             VALUES (?1, 'http://snomed.info/sct', '20240101', 'SNOMED CT', 'active', 'complete', ?2, ?2)",
            rusqlite::params![sys_id, now],
        ).unwrap();

        let concepts = [
            ("000", "Root"),
            ("100", "Disease"),
            ("101", "Infectious Disease"),
            ("102", "Bacterial Disease"),
            ("103", "Viral Disease"),
            ("104", "Chronic Disease"),
            ("105", "Diabetes"),
        ];

        for (code, display) in concepts {
            conn.execute(
                "INSERT INTO concepts (system_id, code, display) VALUES (?1, ?2, ?3)",
                rusqlite::params![sys_id, code, display],
            )
            .unwrap();
        }

        let edges = [
            ("000", "100"),
            ("100", "101"),
            ("100", "104"),
            ("101", "102"),
            ("101", "103"),
            ("104", "105"),
        ];

        for (parent, child) in edges {
            conn.execute(
                "INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![sys_id, parent, child],
            )
            .unwrap();
        }

        drop(conn);
        (backend, sys_id.to_string())
    }

    fn eval(backend: &SqliteTerminologyBackend, system_id: &str, ecl: &str) -> Vec<String> {
        let expr = parser::parse(ecl).expect("parse failed");
        let conn = backend.pool().get().unwrap();
        let mut codes: Vec<String> = evaluate(&conn, system_id, &expr)
            .expect("eval failed")
            .into_iter()
            .map(|c| c.code)
            .collect();
        codes.sort();
        codes
    }

    // ── Exact concept ─────────────────────────────────────────────────────────

    #[test]
    fn exact_concept_returns_single() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "100");
        assert_eq!(codes, vec!["100"]);
    }

    #[test]
    fn exact_concept_not_found_returns_empty() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "999");
        assert!(codes.is_empty());
    }

    // ── Wildcard ──────────────────────────────────────────────────────────────

    #[test]
    fn wildcard_returns_all_concepts() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "*");
        assert_eq!(codes.len(), 7);
    }

    // ── Descendant of (<) ────────────────────────────────────────────────────

    #[test]
    fn descendant_of_disease_does_not_include_self() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "< 100");
        assert!(!codes.contains(&"100".to_string()), "self must be excluded");
        assert!(codes.contains(&"101".to_string()));
        assert!(codes.contains(&"102".to_string()));
        assert!(codes.contains(&"103".to_string()));
        assert!(codes.contains(&"104".to_string()));
        assert!(codes.contains(&"105".to_string()));
        assert_eq!(codes.len(), 5);
    }

    #[test]
    fn descendant_of_leaf_returns_empty() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "< 105");
        assert!(codes.is_empty());
    }

    #[test]
    fn descendant_of_infectious_returns_direct_and_indirect() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "< 101");
        assert_eq!(codes, vec!["102", "103"]);
    }

    // ── Descendant or self (<<) ───────────────────────────────────────────────

    #[test]
    fn descendant_or_self_includes_root() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "<< 100");
        assert!(codes.contains(&"100".to_string()), "self must be included");
        assert_eq!(codes.len(), 6);
    }

    #[test]
    fn descendant_or_self_leaf_returns_only_self() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "<< 105");
        assert_eq!(codes, vec!["105"]);
    }

    // ── Ancestor of (>) ───────────────────────────────────────────────────────

    #[test]
    fn ancestor_of_does_not_include_self() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "> 105");
        assert!(!codes.contains(&"105".to_string()), "self must be excluded");
        assert!(codes.contains(&"104".to_string()));
        assert!(codes.contains(&"100".to_string()));
        assert!(codes.contains(&"000".to_string()));
        assert_eq!(codes.len(), 3);
    }

    #[test]
    fn ancestor_of_root_returns_empty() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "> 000");
        assert!(codes.is_empty());
    }

    // ── Ancestor or self (>>) ─────────────────────────────────────────────────

    #[test]
    fn ancestor_or_self_includes_self() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, ">> 105");
        assert!(codes.contains(&"105".to_string()), "self must be included");
        assert_eq!(codes.len(), 4);
    }

    // ── AND (intersection) ────────────────────────────────────────────────────

    #[test]
    fn and_intersection_overlapping() {
        let (backend, sys_id) = setup_db();
        // descendants of 100 ∩ descendants of 101 = descendants of 101
        let codes = eval(&backend, &sys_id, "< 100 AND < 101");
        assert_eq!(codes, vec!["102", "103"]);
    }

    #[test]
    fn and_intersection_disjoint_returns_empty() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "<< 101 AND << 104");
        assert!(codes.is_empty());
    }

    // ── OR (union) ────────────────────────────────────────────────────────────

    #[test]
    fn or_union_combines_sets() {
        let (backend, sys_id) = setup_db();
        let codes = eval(&backend, &sys_id, "<< 102 OR << 105");
        assert_eq!(codes, vec!["102", "105"]);
    }

    #[test]
    fn or_union_no_duplicates() {
        let (backend, sys_id) = setup_db();
        // Both sides include 100; result must deduplicate.
        let codes = eval(&backend, &sys_id, "<< 100 OR << 100");
        let unique: HashSet<_> = codes.iter().cloned().collect();
        assert_eq!(codes.len(), unique.len(), "duplicates found: {codes:?}");
    }

    // ── MINUS (exclusion) ─────────────────────────────────────────────────────

    #[test]
    fn minus_removes_subtree() {
        let (backend, sys_id) = setup_db();
        // All of 100's descendants minus infectious subtree
        let codes = eval(&backend, &sys_id, "< 100 MINUS << 101");
        // Remaining: 104, 105 (infectious subtree 101/102/103 removed)
        assert!(!codes.contains(&"101".to_string()));
        assert!(!codes.contains(&"102".to_string()));
        assert!(!codes.contains(&"103".to_string()));
        assert!(codes.contains(&"104".to_string()));
        assert!(codes.contains(&"105".to_string()));
    }

    #[test]
    fn minus_removes_nothing_if_disjoint() {
        let (backend, sys_id) = setup_db();
        let left = eval(&backend, &sys_id, "<< 101");
        let codes = eval(&backend, &sys_id, "<< 101 MINUS << 104");
        assert_eq!(codes, left);
    }

    // ── Compound / grouped ────────────────────────────────────────────────────

    #[test]
    fn compound_grouped_expression() {
        let (backend, sys_id) = setup_db();
        // (infectious OR chronic) AND leaf of chronic = diabetes
        let codes = eval(&backend, &sys_id, "(<< 101 OR << 104) AND << 105");
        assert_eq!(codes, vec!["105"]);
    }

    // ── Display values ────────────────────────────────────────────────────────

    #[test]
    fn resolved_concepts_carry_display() {
        let (backend, sys_id) = setup_db();
        let expr = parser::parse("<< 105").unwrap();
        let conn = backend.pool().get().unwrap();
        let results = evaluate(&conn, &sys_id, &expr).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].display.as_deref(), Some("Diabetes"));
    }
}
