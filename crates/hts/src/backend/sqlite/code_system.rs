use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::traits::CodeSystemOperations;
use crate::types::{
    DesignationValue, LookupRequest, LookupResponse, PropertyValue, SubsumesRequest,
    SubsumesResponse, SubsumptionOutcome, ValidateCodeRequest, ValidateCodeResponse,
};

use super::SqliteTerminologyBackend;

#[async_trait]
impl CodeSystemOperations for SqliteTerminologyBackend {
    /// Look up a concept by system URL + code.
    ///
    /// Returns the concept display, all (or filtered) properties, and all
    /// designations. Returns [`HtsError::NotFound`] when either the code
    /// system or the concept does not exist.
    async fn lookup(
        &self,
        _ctx: &TenantContext,
        req: LookupRequest,
    ) -> Result<LookupResponse, HtsError> {
        // SNOMED post-coordination is out of scope for the SQLite MVP.
        if req.expression.is_some() {
            return Err(HtsError::NotSupported(
                "SNOMED post-coordination expressions are not supported in the SQLite backend"
                    .into(),
            ));
        }

        let pool = self.pool().clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            let (system_id, cs_name, cs_version) =
                resolve_code_system(&conn, &req.system, req.version.as_deref())?;

            let (concept_id, display, _definition) = find_concept(&conn, &system_id, &req.code)?;

            let all_props = fetch_properties(&conn, concept_id)?;
            let properties = if req.properties.is_empty() {
                all_props
            } else {
                all_props
                    .into_iter()
                    .filter(|p| req.properties.contains(&p.code))
                    .collect()
            };

            let designations = fetch_designations(&conn, concept_id)?;

            Ok(LookupResponse {
                name: cs_name,
                version: cs_version,
                display,
                properties,
                designations,
            })
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    /// Check whether a code exists in a code system.
    ///
    /// Returns `result=true` and the preferred display when found.
    /// Returns `result=false` (not an error) when the system or code is unknown.
    /// Returns a message when the optional `display` parameter does not match.
    async fn validate_code(
        &self,
        _ctx: &TenantContext,
        req: ValidateCodeRequest,
    ) -> Result<ValidateCodeResponse, HtsError> {
        let system = req.system.clone().ok_or_else(|| {
            HtsError::InvalidRequest(
                "CodeSystem/$validate-code requires 'system' (or 'url') parameter".into(),
            )
        })?;

        let pool = self.pool().clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            // Unknown code system is not an error — just a "false" result.
            let system_id = match resolve_code_system(&conn, &system, req.version.as_deref()) {
                Ok((id, _, _)) => id,
                Err(HtsError::NotFound(_)) => {
                    return Ok(ValidateCodeResponse {
                        result: false,
                        message: Some(format!("Unknown code system: {system}")),
                        display: None,
                    });
                }
                Err(e) => return Err(e),
            };

            // Unknown code is also a "false" result, not an error.
            let display = match find_concept(&conn, &system_id, &req.code) {
                Ok((_, display, _)) => display,
                Err(HtsError::NotFound(_)) => {
                    return Ok(ValidateCodeResponse {
                        result: false,
                        message: Some(format!("Unknown code: {}", req.code)),
                        display: None,
                    });
                }
                Err(e) => return Err(e),
            };

            // Optionally validate the caller's expected display.
            let message = req.display.as_ref().and_then(|expected| {
                let actual = display.as_deref().unwrap_or("");
                if actual != expected.as_str() {
                    Some(format!(
                        "Display mismatch: expected '{}', found '{}'",
                        expected, actual
                    ))
                } else {
                    None
                }
            });

            Ok(ValidateCodeResponse {
                result: true,
                message,
                display,
            })
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    /// Test whether code A subsumes code B within a code system.
    ///
    /// Uses a recursive CTE over the `concept_hierarchy` table so it works
    /// correctly regardless of whether the table stores only direct edges or a
    /// pre-computed transitive closure.
    ///
    /// Returns one of four outcomes:
    /// - `equivalent`   — code_a == code_b
    /// - `subsumes`     — code_a is an ancestor of code_b
    /// - `subsumed-by`  — code_a is a descendant of code_b
    /// - `not-subsumed` — no hierarchical relationship
    async fn subsumes(
        &self,
        _ctx: &TenantContext,
        req: SubsumesRequest,
    ) -> Result<SubsumesResponse, HtsError> {
        let pool = self.pool().clone();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            let (system_id, _, _) =
                resolve_code_system(&conn, &req.system, req.version.as_deref())?;

            // Both codes must exist in this system.
            find_concept(&conn, &system_id, &req.code_a)?;
            find_concept(&conn, &system_id, &req.code_b)?;

            // Same code → equivalent.
            if req.code_a == req.code_b {
                return Ok(SubsumesResponse {
                    outcome: SubsumptionOutcome::Equivalent,
                });
            }

            // Does A subsume B?  (A is an ancestor of B)
            if check_ancestor(&conn, &system_id, &req.code_a, &req.code_b)? {
                return Ok(SubsumesResponse {
                    outcome: SubsumptionOutcome::Subsumes,
                });
            }

            // Is A subsumed by B?  (B is an ancestor of A)
            if check_ancestor(&conn, &system_id, &req.code_b, &req.code_a)? {
                return Ok(SubsumesResponse {
                    outcome: SubsumptionOutcome::SubsumedBy,
                });
            }

            Ok(SubsumesResponse {
                outcome: SubsumptionOutcome::NotSubsumed,
            })
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }
}

// ── Private DB helpers ─────────────────────────────────────────────────────────

/// Return `true` if `ancestor_code` is a (possibly indirect) ancestor of
/// `descendant_code` within the given code system.
///
/// Uses a recursive CTE to walk up the parent chain from `descendant_code`.
/// This handles both pre-computed transitive-closure tables (one lookup) and
/// tables that store only direct parent→child edges (multi-hop traversal).
fn check_ancestor(
    conn: &rusqlite::Connection,
    system_id: &str,
    ancestor_code: &str,
    descendant_code: &str,
) -> Result<bool, HtsError> {
    use rusqlite::OptionalExtension as _;

    let found: Option<i64> = conn
        .query_row(
            "WITH RECURSIVE ancestors(code) AS (
                 SELECT parent_code
                 FROM   concept_hierarchy
                 WHERE  system_id = ?1 AND child_code = ?3
                 UNION
                 SELECT h.parent_code
                 FROM   concept_hierarchy h
                 INNER JOIN ancestors a ON a.code = h.child_code
                 WHERE  h.system_id = ?1
             )
             SELECT 1 FROM ancestors WHERE code = ?2 LIMIT 1",
            rusqlite::params![system_id, ancestor_code, descendant_code],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(found.is_some())
}

/// Resolve a code system by URL (and optionally version).
///
/// Returns `(id, name_or_url, version)`.
fn resolve_code_system(
    conn: &rusqlite::Connection,
    url: &str,
    version: Option<&str>,
) -> Result<(String, String, Option<String>), HtsError> {
    let result = if let Some(ver) = version {
        conn.query_row(
            "SELECT id, COALESCE(name, url), version \
             FROM code_systems WHERE url = ?1 AND version = ?2",
            rusqlite::params![url, ver],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
    } else {
        conn.query_row(
            "SELECT id, COALESCE(name, url), version \
             FROM code_systems WHERE url = ?1",
            rusqlite::params![url],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
    };

    result.map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            HtsError::NotFound(format!("CodeSystem not found: {url}"))
        }
        other => HtsError::StorageError(other.to_string()),
    })
}

/// Look up a concept row by `(system_id, code)`.
///
/// Returns `(concept_id, display, definition)`.
fn find_concept(
    conn: &rusqlite::Connection,
    system_id: &str,
    code: &str,
) -> Result<(i64, Option<String>, Option<String>), HtsError> {
    conn.query_row(
        "SELECT id, display, definition FROM concepts \
         WHERE system_id = ?1 AND code = ?2",
        rusqlite::params![system_id, code],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => {
            HtsError::NotFound(format!("Concept not found: {code}"))
        }
        other => HtsError::StorageError(other.to_string()),
    })
}

/// Fetch all properties for a concept.
fn fetch_properties(
    conn: &rusqlite::Connection,
    concept_id: i64,
) -> Result<Vec<PropertyValue>, HtsError> {
    let mut stmt = conn
        .prepare(
            "SELECT property, value_type, value \
             FROM concept_properties WHERE concept_id = ?1 ORDER BY property",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    stmt.query_map(rusqlite::params![concept_id], |row| {
        Ok(PropertyValue {
            code: row.get(0)?,
            value_type: row.get(1)?,
            value: row.get(2)?,
            description: None,
        })
    })
    .map_err(|e| HtsError::StorageError(e.to_string()))?
    .map(|r| r.map_err(|e| HtsError::StorageError(e.to_string())))
    .collect()
}

/// Fetch all designations for a concept.
fn fetch_designations(
    conn: &rusqlite::Connection,
    concept_id: i64,
) -> Result<Vec<DesignationValue>, HtsError> {
    let mut stmt = conn
        .prepare(
            "SELECT language, use_system, use_code, value \
             FROM concept_designations WHERE concept_id = ?1",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    stmt.query_map(rusqlite::params![concept_id], |row| {
        Ok(DesignationValue {
            language: row.get(0)?,
            use_system: row.get(1)?,
            use_code: row.get(2)?,
            value: row.get(3)?,
        })
    })
    .map_err(|e| HtsError::StorageError(e.to_string()))?
    .map(|r| r.map_err(|e| HtsError::StorageError(e.to_string())))
    .collect()
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::sqlite::SqliteTerminologyBackend;
    use crate::traits::CodeSystemOperations;

    fn backend() -> SqliteTerminologyBackend {
        SqliteTerminologyBackend::in_memory().expect("in-memory DB should initialise")
    }

    fn ctx() -> TenantContext {
        TenantContext::system()
    }

    /// Insert a code system plus one concept with two properties and one designation.
    fn seed(b: &SqliteTerminologyBackend) {
        let conn = b.pool().get().unwrap();
        conn.execute_batch(
            "INSERT INTO code_systems (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('cs1', 'http://example.org/cs', '1.0', 'Example CS', 'active', 'complete',
                     '2024-01-01', '2024-01-01');

             INSERT INTO concepts (id, system_id, code, display, definition)
             VALUES (1, 'cs1', 'ABC', 'Alpha Beta Charlie', 'The definition');

             INSERT INTO concept_properties (concept_id, property, value_type, value)
             VALUES (1, 'parent', 'code', 'ROOT'),
                    (1, 'inactive', 'boolean', 'false');

             INSERT INTO concept_designations (concept_id, language, use_system, use_code, value)
             VALUES (1, 'fr', NULL, NULL, 'Alpha Bêta Charlie');",
        )
        .unwrap();
    }

    // ── $lookup ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn lookup_returns_display_and_properties() {
        let b = backend();
        seed(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs".into(),
                    code: "ABC".into(),
                    version: None,
                    display_language: None,
                    expression: None,
                    properties: vec![],
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.name, "Example CS");
        assert_eq!(resp.version, Some("1.0".into()));
        assert_eq!(resp.display, Some("Alpha Beta Charlie".into()));
        assert_eq!(resp.properties.len(), 2);
        assert_eq!(resp.designations.len(), 1);
        assert_eq!(resp.designations[0].language, Some("fr".into()));
    }

    #[tokio::test]
    async fn lookup_property_filter() {
        let b = backend();
        seed(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs".into(),
                    code: "ABC".into(),
                    version: None,
                    display_language: None,
                    expression: None,
                    properties: vec!["parent".into()],
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.properties.len(), 1);
        assert_eq!(resp.properties[0].code, "parent");
        assert_eq!(resp.properties[0].value, "ROOT");
    }

    #[tokio::test]
    async fn lookup_unknown_code_returns_not_found() {
        let b = backend();
        seed(&b);

        let err = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs".into(),
                    code: "NOPE".into(),
                    version: None,
                    display_language: None,
                    expression: None,
                    properties: vec![],
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }

    #[tokio::test]
    async fn lookup_unknown_system_returns_not_found() {
        let b = backend();
        seed(&b);

        let err = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://other.org/cs".into(),
                    code: "ABC".into(),
                    version: None,
                    display_language: None,
                    expression: None,
                    properties: vec![],
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }

    #[tokio::test]
    async fn lookup_expression_returns_not_supported() {
        let b = backend();

        let err = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs".into(),
                    code: "ABC".into(),
                    version: None,
                    display_language: None,
                    expression: Some("128045006:{363698007=56459004}".into()),
                    properties: vec![],
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotSupported(_)));
    }

    // ── $validate-code ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn validate_code_valid_code() {
        let b = backend();
        seed(&b);

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: None,
                    system: Some("http://example.org/cs".into()),
                    code: "ABC".into(),
                    version: None,
                    display: None,
                },
            )
            .await
            .unwrap();

        assert!(resp.result);
        assert_eq!(resp.display, Some("Alpha Beta Charlie".into()));
        assert!(resp.message.is_none());
    }

    #[tokio::test]
    async fn validate_code_unknown_code_returns_false() {
        let b = backend();
        seed(&b);

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: None,
                    system: Some("http://example.org/cs".into()),
                    code: "NOPE".into(),
                    version: None,
                    display: None,
                },
            )
            .await
            .unwrap();

        assert!(!resp.result);
        assert!(resp.message.is_some());
    }

    #[tokio::test]
    async fn validate_code_unknown_system_returns_false() {
        let b = backend();
        seed(&b);

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: None,
                    system: Some("http://unknown.org/cs".into()),
                    code: "ABC".into(),
                    version: None,
                    display: None,
                },
            )
            .await
            .unwrap();

        assert!(!resp.result);
    }

    #[tokio::test]
    async fn validate_code_display_match() {
        let b = backend();
        seed(&b);

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: None,
                    system: Some("http://example.org/cs".into()),
                    code: "ABC".into(),
                    version: None,
                    display: Some("Alpha Beta Charlie".into()),
                },
            )
            .await
            .unwrap();

        assert!(resp.result);
        assert!(resp.message.is_none(), "no mismatch expected");
    }

    #[tokio::test]
    async fn validate_code_display_mismatch_still_valid_but_has_message() {
        let b = backend();
        seed(&b);

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: None,
                    system: Some("http://example.org/cs".into()),
                    code: "ABC".into(),
                    version: None,
                    display: Some("Wrong Display".into()),
                },
            )
            .await
            .unwrap();

        assert!(resp.result, "code itself is valid");
        assert!(
            resp.message.is_some(),
            "display mismatch should produce a message"
        );
    }

    #[tokio::test]
    async fn validate_code_missing_system_returns_error() {
        let b = backend();

        let err = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: None,
                    system: None,
                    code: "ABC".into(),
                    version: None,
                    display: None,
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::InvalidRequest(_)));
    }

    // ── $subsumes ──────────────────────────────────────────────────────────────

    /// Seed a three-level hierarchy:  A → B → C
    ///
    /// Direct edges only — the recursive CTE in `check_ancestor` must
    /// infer the transitive link A → C at query time.
    fn seed_hierarchy(b: &SqliteTerminologyBackend) {
        let conn = b.pool().get().unwrap();
        conn.execute_batch(
            "INSERT INTO code_systems (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('cs2', 'http://example.org/hier', '1.0', 'Hierarchy CS', 'active', 'complete',
                     '2024-01-01', '2024-01-01');

             INSERT INTO concepts (id, system_id, code, display)
             VALUES (10, 'cs2', 'A', 'Concept A'),
                    (11, 'cs2', 'B', 'Concept B'),
                    (12, 'cs2', 'C', 'Concept C'),
                    (13, 'cs2', 'D', 'Concept D');

             -- Direct edges: A is parent of B; B is parent of C.
             -- D is a sibling with no relationship to A/B/C.
             INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
             VALUES ('cs2', 'A', 'B'),
                    ('cs2', 'B', 'C');",
        )
        .unwrap();
    }

    fn req(code_a: &str, code_b: &str) -> SubsumesRequest {
        SubsumesRequest {
            system: "http://example.org/hier".into(),
            version: None,
            code_a: code_a.into(),
            code_b: code_b.into(),
        }
    }

    #[tokio::test]
    async fn subsumes_equivalent_same_code() {
        let b = backend();
        seed_hierarchy(&b);

        let resp = b.subsumes(&ctx(), req("A", "A")).await.unwrap();
        assert_eq!(resp.outcome, SubsumptionOutcome::Equivalent);
    }

    #[tokio::test]
    async fn subsumes_direct_parent_child() {
        let b = backend();
        seed_hierarchy(&b);

        // A is the direct parent of B → A subsumes B.
        let resp = b.subsumes(&ctx(), req("A", "B")).await.unwrap();
        assert_eq!(resp.outcome, SubsumptionOutcome::Subsumes);
    }

    #[tokio::test]
    async fn subsumes_transitive_ancestor() {
        let b = backend();
        seed_hierarchy(&b);

        // A → B → C: A is an indirect ancestor of C.
        let resp = b.subsumes(&ctx(), req("A", "C")).await.unwrap();
        assert_eq!(resp.outcome, SubsumptionOutcome::Subsumes);
    }

    #[tokio::test]
    async fn subsumes_subsumed_by_direct() {
        let b = backend();
        seed_hierarchy(&b);

        // B is a direct child of A → B is subsumed-by A.
        let resp = b.subsumes(&ctx(), req("B", "A")).await.unwrap();
        assert_eq!(resp.outcome, SubsumptionOutcome::SubsumedBy);
    }

    #[tokio::test]
    async fn subsumes_subsumed_by_transitive() {
        let b = backend();
        seed_hierarchy(&b);

        // C → B → A: C is a transitive descendant of A.
        let resp = b.subsumes(&ctx(), req("C", "A")).await.unwrap();
        assert_eq!(resp.outcome, SubsumptionOutcome::SubsumedBy);
    }

    #[tokio::test]
    async fn subsumes_not_subsumed_unrelated() {
        let b = backend();
        seed_hierarchy(&b);

        // D has no relationship to A.
        let resp = b.subsumes(&ctx(), req("A", "D")).await.unwrap();
        assert_eq!(resp.outcome, SubsumptionOutcome::NotSubsumed);
    }

    #[tokio::test]
    async fn subsumes_not_subsumed_siblings() {
        let b = backend();
        seed_hierarchy(&b);

        // B and D share no hierarchy.
        let resp = b.subsumes(&ctx(), req("B", "D")).await.unwrap();
        assert_eq!(resp.outcome, SubsumptionOutcome::NotSubsumed);
    }

    #[tokio::test]
    async fn subsumes_unknown_system_returns_not_found() {
        let b = backend();

        let err = b
            .subsumes(
                &ctx(),
                SubsumesRequest {
                    system: "http://unknown.org/cs".into(),
                    version: None,
                    code_a: "A".into(),
                    code_b: "B".into(),
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }

    #[tokio::test]
    async fn subsumes_unknown_code_a_returns_not_found() {
        let b = backend();
        seed_hierarchy(&b);

        let err = b
            .subsumes(
                &ctx(),
                SubsumesRequest {
                    system: "http://example.org/hier".into(),
                    version: None,
                    code_a: "NOPE".into(),
                    code_b: "A".into(),
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }

    #[tokio::test]
    async fn subsumes_unknown_code_b_returns_not_found() {
        let b = backend();
        seed_hierarchy(&b);

        let err = b
            .subsumes(
                &ctx(),
                SubsumesRequest {
                    system: "http://example.org/hier".into(),
                    version: None,
                    code_a: "A".into(),
                    code_b: "NOPE".into(),
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }
}
