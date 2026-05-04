//! SQLite implementation of [`CodeSystemOperations`].
//!
//! Implements `$lookup`, `$validate-code`, and `$subsumes` against the
//! `code_systems`, `concepts`, `concept_properties`, `concept_designations`,
//! and `concept_hierarchy` tables.
//!
//! Subsumption is O(1): the `concept_hierarchy` table is pre-materialised at
//! import time, so ancestor/descendant checks become direct closure lookups
//! instead of recursive traversals.  SNOMED post-coordination expressions are
//! deliberately unsupported and return [`HtsError::NotSupported`].

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use rusqlite::OptionalExtension;

use crate::error::HtsError;
use crate::traits::{CodeSystemOperations, ConceptDesignation, ConceptExpansionFlags};
use crate::types::{
    DesignationValue, LookupRequest, LookupResponse, PropertyValue, ResourceSearchQuery,
    SubsumesRequest, SubsumesResponse, SubsumptionOutcome, ValidateCodeRequest,
    ValidateCodeResponse,
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

            let (system_id, cs_name, cs_version) = resolve_code_system(
                &conn,
                &req.system,
                req.version.as_deref(),
                req.date.as_deref(),
            )?;

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

            let all_designations = fetch_designations(&conn, concept_id)?;

            // 10.2: When displayLanguage is set and a matching designation
            // exists, prefer its value as the concept display.
            let display = if let Some(lang) = req.display_language.as_deref() {
                all_designations
                    .iter()
                    .find(|d| d.language.as_deref() == Some(lang))
                    .map(|d| d.value.clone())
                    .or(display)
            } else {
                display
            };

            // 10.1: Filter designations to the requested language (if set).
            let designations = if let Some(lang) = req.display_language.as_deref() {
                all_designations
                    .into_iter()
                    .filter(|d| d.language.as_deref() == Some(lang))
                    .collect()
            } else {
                all_designations
            };

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
            let system_id = match resolve_code_system(
                &conn,
                &system,
                req.version.as_deref(),
                req.date.as_deref(),
            ) {
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
            // Per FHIR spec, a display mismatch causes result=false (with a message).
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
                result: message.is_none(),
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
                resolve_code_system(&conn, &req.system, req.version.as_deref(), None)?;

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

    async fn code_system_version_for_url(
        &self,
        _ctx: &TenantContext,
        url: &str,
    ) -> Result<Option<String>, HtsError> {
        let pool = self.pool().clone();
        let url = url.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
            let version: Option<String> = conn
                .query_row(
                    "SELECT version FROM code_systems WHERE url = ?1 LIMIT 1",
                    rusqlite::params![url],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| HtsError::StorageError(e.to_string()))?
                .flatten();
            Ok(version)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    async fn concept_designations(
        &self,
        _ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<ConceptDesignation>>, HtsError> {
        if codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let pool = self.pool().clone();
        let system_url = system_url.to_string();
        let codes = codes.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            let placeholders = (2..=codes.len() + 1)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT c.code, cd.language, cd.use_system, cd.use_code, cd.value
                 FROM concept_designations cd
                 JOIN concepts c ON c.id = cd.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = ?1
                   AND c.code IN ({placeholders})",
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(codes.len() + 1);
            params.push(&system_url);
            for c in &codes {
                params.push(c as &dyn rusqlite::ToSql);
            }

            let mut out: std::collections::HashMap<String, Vec<ConceptDesignation>> =
                std::collections::HashMap::new();
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            for r in rows {
                let (code, language, use_system, use_code, value) =
                    r.map_err(|e| HtsError::StorageError(e.to_string()))?;
                out.entry(code).or_default().push(ConceptDesignation {
                    language,
                    use_system,
                    use_code,
                    value,
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    async fn concept_expansion_flags(
        &self,
        _ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, ConceptExpansionFlags>, HtsError> {
        if codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let pool = self.pool().clone();
        let system_url = system_url.to_string();
        let codes = codes.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            // Build a parameterised IN-list: (?2, ?3, ...). The first param is
            // the system URL.
            let placeholders = (2..=codes.len() + 1)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT c.code, cp.property, cp.value
                 FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = ?1
                   AND c.code IN ({placeholders})
                   AND cp.property IN ('notSelectable', 'status')",
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(codes.len() + 1);
            params.push(&system_url);
            for c in &codes {
                params.push(c as &dyn rusqlite::ToSql);
            }

            let mut out: std::collections::HashMap<String, ConceptExpansionFlags> =
                std::collections::HashMap::new();
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            for r in rows {
                let (code, property, value) =
                    r.map_err(|e| HtsError::StorageError(e.to_string()))?;
                let flags = out.entry(code).or_default();
                match property.as_str() {
                    "notSelectable" if value == "true" => flags.is_abstract = true,
                    "status"
                        if matches!(value.as_str(), "retired" | "deprecated" | "withdrawn") =>
                    {
                        flags.inactive = true;
                    }
                    _ => {}
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    /// Search CodeSystem resources by query parameters.
    ///
    /// Filters are applied as exact matches against stored columns. Omitting a
    /// field means "no filter". Returns up to `count` results starting at
    /// `offset`, defaulting to 20 results from the beginning.
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
            // idx_code_systems_meta serves this query without touching the main table.
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
                         FROM code_systems
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
                    results.push(build_synthetic_resource(
                        "CodeSystem",
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
                     FROM code_systems
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
                        build_synthetic_resource(
                            "CodeSystem",
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

// ── Private DB helpers ─────────────────────────────────────────────────────────

/// Return `true` if `ancestor_code` is a (possibly indirect) ancestor of
/// `descendant_code` within the given code system.
///
/// O(1) PRIMARY KEY lookup against the precomputed `concept_closure` table.
/// Self-links are stored in the closure, so `ancestor_code == descendant_code`
/// returns `true`.
fn check_ancestor(
    conn: &rusqlite::Connection,
    system_id: &str,
    ancestor_code: &str,
    descendant_code: &str,
) -> Result<bool, HtsError> {
    use rusqlite::OptionalExtension as _;

    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM concept_closure
             WHERE system_id = ?1 AND ancestor_code = ?2 AND descendant_code = ?3
             LIMIT 1",
            rusqlite::params![system_id, ancestor_code, descendant_code],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(found.is_some())
}

/// Resolve a code system by URL, optional version, and optional point-in-time date.
///
/// Returns `(id, name_or_url, version)`.
///
/// When `date` is provided, only code systems whose `$.date` (from `resource_json`)
/// is ≤ the requested date are matched, enabling point-in-time evaluation.
fn resolve_code_system(
    conn: &rusqlite::Connection,
    url: &str,
    version: Option<&str>,
    date: Option<&str>,
) -> Result<(String, String, Option<String>), HtsError> {
    let no_version_query = |conn: &rusqlite::Connection| {
        conn.query_row(
            "SELECT id, COALESCE(name, url), version \
             FROM code_systems \
             WHERE url = ?1 \
               AND (?2 IS NULL OR json_extract(resource_json, '$.date') <= ?2)",
            rusqlite::params![url, date],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
    };

    let result = if let Some(ver) = version {
        // Try exact version first; fall back to any version of the same URL so
        // clients that pass an informational version string (e.g. the pool was
        // generated from v3-ActCode "9.0.0" but we have "2.1.0" imported) still
        // get a result rather than a spurious 404.
        let exact = conn.query_row(
            "SELECT id, COALESCE(name, url), version \
             FROM code_systems \
             WHERE url = ?1 AND version = ?2 \
               AND (?3 IS NULL OR json_extract(resource_json, '$.date') <= ?3)",
            rusqlite::params![url, ver, date],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        );
        match exact {
            Err(rusqlite::Error::QueryReturnedNoRows) => no_version_query(conn),
            other => other,
        }
    } else {
        no_version_query(conn)
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
        .prepare_cached(
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
        .prepare_cached(
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

/// Build a minimal synthetic FHIR resource JSON when `resource_json` is absent.
///
/// Used as a fallback for resources that pre-date the `resource_json` column.
pub(super) fn build_synthetic_resource(
    resource_type: &str,
    id: &str,
    url: &str,
    version: Option<&str>,
    name: Option<&str>,
    title: Option<&str>,
    status: &str,
) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "resourceType": resource_type,
        "id": id,
        "url": url,
        "status": status,
    });
    if let Some(v) = version {
        obj["version"] = v.into();
    }
    if let Some(n) = name {
        obj["name"] = n.into();
    }
    if let Some(t) = title {
        obj["title"] = t.into();
    }
    obj
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::sqlite::SqliteTerminologyBackend;
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
                    properties: vec![],
                    ..Default::default()
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
                    properties: vec!["parent".into()],
                    ..Default::default()
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
                    ..Default::default()
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
                    ..Default::default()
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
                    expression: Some("128045006:{363698007=56459004}".into()),
                    ..Default::default()
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
                    system: Some("http://example.org/cs".into()),
                    code: "ABC".into(),
                    ..Default::default()
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
                    system: Some("http://example.org/cs".into()),
                    code: "NOPE".into(),
                    ..Default::default()
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
                    system: Some("http://unknown.org/cs".into()),
                    code: "ABC".into(),
                    ..Default::default()
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
                    system: Some("http://example.org/cs".into()),
                    code: "ABC".into(),
                    display: Some("Alpha Beta Charlie".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(resp.result);
        assert!(resp.message.is_none(), "no mismatch expected");
    }

    #[tokio::test]
    async fn validate_code_display_mismatch_returns_false_with_message() {
        let b = backend();
        seed(&b);

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    system: Some("http://example.org/cs".into()),
                    code: "ABC".into(),
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
                    code: "ABC".into(),
                    ..Default::default()
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
        crate::backends::sqlite::schema::build_concept_closure(&conn, "cs2").unwrap();
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

    // ── date parameter (point-in-time filtering) ───────────────────────────────

    /// Seed a code system whose `resource_json` contains a `date` field.
    fn seed_with_date(b: &SqliteTerminologyBackend, cs_date: &str) {
        let conn = b.pool().get().unwrap();
        let resource_json = serde_json::json!({
            "resourceType": "CodeSystem",
            "id": "cs-dated",
            "url": "http://example.org/cs-dated",
            "name": "DatedCS",
            "status": "active",
            "date": cs_date
        })
        .to_string();
        conn.execute_batch(&format!(
            "INSERT INTO code_systems
                 (id, url, version, name, status, content, resource_json, created_at, updated_at)
             VALUES ('cs-dated', 'http://example.org/cs-dated', NULL, 'DatedCS',
                     'active', 'complete', '{resource_json}', '2024-01-01', '2024-01-01');
             INSERT INTO concepts (id, system_id, code, display)
             VALUES (99, 'cs-dated', 'TEST', 'Test Concept');",
        ))
        .unwrap();
    }

    #[tokio::test]
    async fn lookup_date_after_cs_date_succeeds() {
        let b = backend();
        seed_with_date(&b, "2024-06-01");

        // Request date is after the CS date → code system is in scope.
        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs-dated".into(),
                    code: "TEST".into(),
                    date: Some("2024-12-31".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.display, Some("Test Concept".into()));
    }

    #[tokio::test]
    async fn lookup_date_before_cs_date_returns_not_found() {
        let b = backend();
        seed_with_date(&b, "2024-06-01");

        // Request date is before the CS date → code system excluded.
        let err = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs-dated".into(),
                    code: "TEST".into(),
                    date: Some("2024-01-01".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, HtsError::NotFound(_)));
    }

    #[tokio::test]
    async fn lookup_without_date_ignores_cs_date_field() {
        let b = backend();
        seed_with_date(&b, "2024-06-01");

        // No date param → date filter is NULL → all code systems match.
        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs-dated".into(),
                    code: "TEST".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.display, Some("Test Concept".into()));
    }

    // ── Phase 10: displayLanguage filtering ───────────────────────────────────

    fn seed_multilang(b: &SqliteTerminologyBackend) {
        let conn = b.pool().get().unwrap();
        conn.execute_batch(
            "INSERT INTO code_systems
                 (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('cs-ml', 'http://example.org/cs-ml', '1.0', 'Multilang CS',
                     'active', 'complete', '2024-01-01', '2024-01-01');

             INSERT INTO concepts (id, system_id, code, display)
             VALUES (100, 'cs-ml', 'TERM', 'Term (English default)');

             INSERT INTO concept_designations (concept_id, language, use_system, use_code, value)
             VALUES (100, 'en', NULL, NULL, 'Term in English'),
                    (100, 'fr', NULL, NULL, 'Terme en français'),
                    (100, 'de', NULL, NULL, 'Begriff auf Deutsch');",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn lookup_display_language_filters_designations() {
        let b = backend();
        seed_multilang(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs-ml".into(),
                    code: "TERM".into(),
                    display_language: Some("fr".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Only the French designation should be returned.
        assert_eq!(resp.designations.len(), 1);
        assert_eq!(resp.designations[0].language.as_deref(), Some("fr"));
        assert_eq!(resp.designations[0].value, "Terme en français");
    }

    #[tokio::test]
    async fn lookup_display_language_overrides_default_display() {
        let b = backend();
        seed_multilang(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs-ml".into(),
                    code: "TERM".into(),
                    display_language: Some("fr".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Display should be the French designation value, not the default English display.
        assert_eq!(resp.display, Some("Terme en français".into()));
    }

    #[tokio::test]
    async fn lookup_display_language_no_match_returns_empty_designations() {
        let b = backend();
        seed_multilang(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs-ml".into(),
                    code: "TERM".into(),
                    display_language: Some("zh".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // No Chinese designations — filtered list is empty.
        assert!(resp.designations.is_empty());
        // Display falls back to concept default.
        assert_eq!(resp.display, Some("Term (English default)".into()));
    }

    #[tokio::test]
    async fn lookup_without_display_language_returns_all_designations() {
        let b = backend();
        seed_multilang(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs-ml".into(),
                    code: "TERM".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // All three designations returned when no filter is applied.
        assert_eq!(resp.designations.len(), 3);
        // Default display is unchanged.
        assert_eq!(resp.display, Some("Term (English default)".into()));
    }
}
