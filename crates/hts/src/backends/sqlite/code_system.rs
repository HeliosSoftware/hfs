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
use crate::traits::{
    CodeSystemOperations, ConceptDesignation, ConceptExpansionFlags, SupplementInfo,
};
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

            let (concept_id, display, definition) = find_concept(&conn, &system_id, &req.code)?;

            let stored_props = fetch_properties(&conn, concept_id)?;
            // Per FHIR spec, property="*" is the wildcard meaning "include
            // every property the concept has". Treat any "*" entry as
            // equivalent to omitting the filter.
            let want_all = req.properties.is_empty() || req.properties.iter().any(|p| p == "*");

            // Synthesised properties (parent/child/inactive) are derived from
            // the hierarchy and status tables rather than concept_properties.
            // Most callers (and the tx-ecosystem IG fixtures) expect these to
            // appear alongside the stored properties when property=* or any
            // explicit filter names them.
            let synth_props =
                fetch_synthesised_properties(&conn, &system_id, &req.code, &stored_props)?;

            let properties = if want_all {
                let mut out = stored_props;
                out.extend(synth_props);
                out
            } else {
                let mut out: Vec<PropertyValue> = stored_props
                    .into_iter()
                    .filter(|p| req.properties.contains(&p.code))
                    .collect();
                out.extend(
                    synth_props
                        .into_iter()
                        .filter(|p| req.properties.contains(&p.code)),
                );
                out
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
                definition,
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
                    let text = format!(
                        "A definition for CodeSystem {system} could not be found, so the code cannot be validated"
                    );
                    return Ok(ValidateCodeResponse {
                        result: false,
                        message: Some(text.clone()),
                        display: None,
                        inactive: None,
                        issues: vec![crate::types::ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "not-found".into(),
                            tx_code: "not-found".into(),
                            text,
                            location: Some("Coding.system".into()),
                            message_id: Some("UNKNOWN_CODESYSTEM".into()),
                        }],
                    });
                }
                Err(e) => return Err(e),
            };

            // Unknown code is also a "false" result, not an error.
            let display = match find_concept(&conn, &system_id, &req.code) {
                Ok((_, display, _)) => display,
                Err(HtsError::NotFound(_)) => {
                    let text = format!(
                        "Unknown_Code_in_Version: The code '{}' is not valid in the system {}",
                        req.code, system
                    );
                    return Ok(ValidateCodeResponse {
                        result: false,
                        message: Some(text.clone()),
                        display: None,
                        inactive: None,
                        issues: vec![crate::types::ValidationIssue {
                            severity: "error".into(),
                            fhir_code: "code-invalid".into(),
                            tx_code: "invalid-code".into(),
                            text,
                            location: Some("Coding.code".into()),
                            message_id: Some("Unknown_Code_in_Version".into()),
                        }],
                    });
                }
                Err(e) => return Err(e),
            };

            // Optionally validate the caller's expected display.
            // Per FHIR spec, a display mismatch causes result=false (with a message).
            let mut issues: Vec<crate::types::ValidationIssue> = Vec::new();
            let message = req.display.as_ref().and_then(|expected| {
                let actual = display.as_deref().unwrap_or("");
                if actual != expected.as_str() {
                    let text = format!(
                        "Display mismatch: expected '{}', found '{}'",
                        expected, actual
                    );
                    issues.push(crate::types::ValidationIssue {
                        severity: "error".into(),
                        fhir_code: "invalid".into(),
                        tx_code: "invalid-display".into(),
                        text: text.clone(),
                        location: Some("Coding.display".into()),
                        message_id: Some(
                            "Display_Name_for__should_be_one_of__instead_of".into(),
                        ),
                    });
                    Some(text)
                } else {
                    None
                }
            });

            Ok(ValidateCodeResponse {
                result: message.is_none(),
                message,
                display,
                inactive: None,
                issues,
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
                    source: None,
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    async fn concept_property_values(
        &self,
        _ctx: &TenantContext,
        system_url: &str,
        codes: &[String],
        properties: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, HtsError> {
        if codes.is_empty() || properties.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let pool = self.pool().clone();
        let system_url = system_url.to_string();
        let codes = codes.to_vec();
        let properties = properties.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
            let code_ph = (2..=codes.len() + 1)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let prop_ph = (codes.len() + 2..=codes.len() + properties.len() + 1)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT c.code, cp.property, cp.value
                 FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = ?1
                   AND c.code IN ({code_ph})
                   AND cp.property IN ({prop_ph})",
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let mut params: Vec<&dyn rusqlite::ToSql> =
                Vec::with_capacity(1 + codes.len() + properties.len());
            params.push(&system_url);
            for c in &codes {
                params.push(c as &dyn rusqlite::ToSql);
            }
            for p in &properties {
                params.push(p as &dyn rusqlite::ToSql);
            }
            let mut out: std::collections::HashMap<String, Vec<(String, String)>> =
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
                let (code, prop, value) = r.map_err(|e| HtsError::StorageError(e.to_string()))?;
                out.entry(code).or_default().push((prop, value));
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

            let placeholders = (2..=codes.len() + 1)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            // Per FHIR concept-properties IG, the standard `notSelectable`
            // and `inactive` properties' local CodeSystem.property.code can
            // be ANY local name (e.g. `not-selectable` with a hyphen in the
            // tx-ecosystem `notSelectable/` fixtures). Resolve via uri →
            // local-code mapping when available; always fall back to the
            // canonical names so a CS that never declares property[] still
            // reports correctly.
            let abstract_codes = abstract_property_codes(&conn, &system_url);
            let inactive_codes = inactive_property_codes(&conn, &system_url);
            let abstract_in = abstract_codes
                .iter()
                .map(|c| format!("'{}'", c.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            let inactive_in = inactive_codes
                .iter()
                .map(|c| format!("'{}'", c.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT c.code, cp.property, cp.value
                 FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = ?1
                   AND c.code IN ({placeholders})
                   AND (
                       (cp.property IN ({abstract_in}) AND cp.value = 'true')
                    OR (cp.property IN ({inactive_in}) AND cp.value = 'true')
                    OR (cp.property = 'status'
                        AND cp.value IN ('retired', 'deprecated', 'withdrawn', 'inactive'))
                   )"
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
                let (code, property, _value) =
                    r.map_err(|e| HtsError::StorageError(e.to_string()))?;
                let flags = out.entry(code).or_default();
                if property == "status" || inactive_codes.iter().any(|c| c == &property) {
                    flags.inactive = true;
                } else if abstract_codes.iter().any(|c| c == &property) {
                    flags.is_abstract = true;
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

    // ── Supplements ──────────────────────────────────────────────────────────
    //
    // Supplements are stored in the same `code_systems` table as any other
    // CodeSystem; the only distinguishing fields are `content='supplement'`
    // and a `supplements` field on the resource_json pointing at the URL of
    // the base CS being modified. We deliberately do NOT add a column to the
    // schema for the supplement target — the value lives in `resource_json`
    // and is read on demand. This keeps the schema migration-free.
    async fn supplement_target(
        &self,
        _ctx: &TenantContext,
        supplement_url: &str,
    ) -> Result<Option<SupplementInfo>, HtsError> {
        let pool = self.pool().clone();
        let supplement_url = supplement_url.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
            // Read content + resource_json + version in one query so we can
            // confirm the row really is a supplement before returning.
            let row: Option<(String, Option<String>, Option<String>)> = conn
                .query_row(
                    "SELECT content, version, json_extract(resource_json, '$.supplements')
                     FROM code_systems
                     WHERE url = ?1
                     LIMIT 1",
                    rusqlite::params![supplement_url],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let Some((content, version, target)) = row else {
                return Ok(None);
            };
            if content != "supplement" {
                return Ok(None);
            }
            let target_url = match target {
                Some(t) => t,
                None => return Ok(None),
            };
            let supplement_canonical = match version {
                Some(v) => format!("{supplement_url}|{v}"),
                None => supplement_url.clone(),
            };
            Ok(Some(SupplementInfo {
                target_url,
                supplement_canonical,
            }))
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    async fn supplement_designations(
        &self,
        _ctx: &TenantContext,
        supplement_urls: &[String],
        codes: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<ConceptDesignation>>, HtsError> {
        if supplement_urls.is_empty() || codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let pool = self.pool().clone();
        let supplement_urls = supplement_urls.to_vec();
        let codes = codes.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

            // Two IN-clauses: one for the supplement URL set, one for the
            // code set. We also pull s.url and s.version so the response can
            // report `source = "url|version"`.
            let url_ph = (1..=supplement_urls.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let code_ph = (supplement_urls.len() + 1..=supplement_urls.len() + codes.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT c.code, cd.language, cd.use_system, cd.use_code, cd.value,
                        s.url, s.version
                 FROM concept_designations cd
                 JOIN concepts c ON c.id = cd.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url IN ({url_ph})
                   AND s.content = 'supplement'
                   AND c.code IN ({code_ph})",
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let mut params: Vec<&dyn rusqlite::ToSql> =
                Vec::with_capacity(supplement_urls.len() + codes.len());
            for u in &supplement_urls {
                params.push(u as &dyn rusqlite::ToSql);
            }
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
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            for r in rows {
                let (code, language, use_system, use_code, value, supp_url, supp_ver) =
                    r.map_err(|e| HtsError::StorageError(e.to_string()))?;
                let source = match supp_ver {
                    Some(v) => format!("{supp_url}|{v}"),
                    None => supp_url,
                };
                out.entry(code).or_default().push(ConceptDesignation {
                    language,
                    use_system,
                    use_code,
                    value,
                    source: Some(source),
                });
            }
            Ok(out)
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }

    async fn supplement_property_values(
        &self,
        _ctx: &TenantContext,
        supplement_urls: &[String],
        codes: &[String],
        properties: &[String],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, HtsError> {
        if supplement_urls.is_empty() || codes.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        // Empty `properties` slice = "every property defined on the
        // matching supplement concepts". Used by lookup wildcard mode.
        let want_all_props = properties.is_empty();
        let pool = self.pool().clone();
        let supplement_urls = supplement_urls.to_vec();
        let codes = codes.to_vec();
        let properties = properties.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
            let mut idx = 1usize;
            let url_ph = (idx..idx + supplement_urls.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            idx += supplement_urls.len();
            let code_ph = (idx..idx + codes.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            idx += codes.len();
            let prop_clause = if want_all_props {
                String::new()
            } else {
                let prop_ph = (idx..idx + properties.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(" AND cp.property IN ({prop_ph})")
            };
            let sql = format!(
                "SELECT c.code, cp.property, cp.value
                 FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url IN ({url_ph})
                   AND s.content = 'supplement'
                   AND c.code IN ({code_ph})
                   {prop_clause}",
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            let mut params: Vec<&dyn rusqlite::ToSql> =
                Vec::with_capacity(supplement_urls.len() + codes.len() + properties.len());
            for u in &supplement_urls {
                params.push(u as &dyn rusqlite::ToSql);
            }
            for c in &codes {
                params.push(c as &dyn rusqlite::ToSql);
            }
            if !want_all_props {
                for p in &properties {
                    params.push(p as &dyn rusqlite::ToSql);
                }
            }
            let mut out: std::collections::HashMap<String, Vec<(String, String)>> =
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
                let (code, prop, value) = r.map_err(|e| HtsError::StorageError(e.to_string()))?;
                out.entry(code).or_default().push((prop, value));
            }
            Ok(out)
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
/// Resolve the local property code(s) that map to the FHIR `notSelectable`
/// concept-property URI in `system_url`'s CodeSystem definition. Tx-ecosystem
/// fixtures may rename it locally (e.g. `not-selectable` with a hyphen) so the
/// concept_expansion_flags lookup needs to know which property name(s) on this
/// CS encode the abstract flag. Always includes the canonical names as a safety
/// net for systems that didn't declare property[].
pub(super) fn abstract_property_codes(
    conn: &rusqlite::Connection,
    system_url: &str,
) -> Vec<String> {
    cs_property_local_codes(conn, system_url, "notSelectable")
}

/// Same idea as [`abstract_property_codes`] but for the FHIR
/// `http://hl7.org/fhir/concept-properties#inactive` property — used by
/// $expand to populate `contains[].inactive` and by $validate-code to
/// flag inactive codes. Always includes the canonical `inactive` name.
pub(super) fn inactive_property_codes(
    conn: &rusqlite::Connection,
    system_url: &str,
) -> Vec<String> {
    cs_property_local_codes(conn, system_url, "inactive")
}

/// Resolve the local property code(s) on `system_url`'s CodeSystem that
/// declare a `uri` ending in `#<canonical>` (or `<canonical>` exactly).
/// Always includes `<canonical>` as a fallback so a CS that didn't declare
/// `property[]` still reports correctly.
fn cs_property_local_codes(
    conn: &rusqlite::Connection,
    system_url: &str,
    canonical: &str,
) -> Vec<String> {
    let mut codes: Vec<String> = vec![canonical.to_string()];
    let resource_json: Option<String> = conn
        .query_row(
            "SELECT resource_json FROM code_systems \
             WHERE url = ?1 \
             ORDER BY COALESCE(version, '') DESC LIMIT 1",
            rusqlite::params![system_url],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();
    let suffix = format!("#{canonical}");
    if let Some(json) = resource_json {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            if let Some(props) = v.get("property").and_then(|p| p.as_array()) {
                for p in props {
                    let uri = p.get("uri").and_then(|u| u.as_str()).unwrap_or("");
                    if uri.ends_with(&suffix) || uri == canonical {
                        if let Some(local_code) = p.get("code").and_then(|c| c.as_str()) {
                            if !codes.iter().any(|c| c == local_code) {
                                codes.push(local_code.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    codes
}

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
/// Version-matching rules (mirroring tx.fhir.org behaviour exercised by the
/// tx-ecosystem `version/` test suite):
///
/// * `Some("1.x.x")` / `Some("1.x")` / `Some("1")` — partial match.  Each `x`
///   segment acts as a wildcard, so `1.x.x` matches `1.0.0`, `1.2.0`, etc.
///   The highest matching version wins.
/// * `Some("1.0.0")` — exact match required.
/// * `None` — no version pinning; the row with the highest `version` (sorted
///   descending as text) wins so callers default to the most recent revision.
///
/// When `date` is provided, only code systems whose `$.date` (from
/// `resource_json`) is ≤ the requested date are considered.
fn resolve_code_system(
    conn: &rusqlite::Connection,
    url: &str,
    version: Option<&str>,
    date: Option<&str>,
) -> Result<(String, String, Option<String>), HtsError> {
    let candidates = fetch_versions(conn, url, date)?;
    if candidates.is_empty() {
        return Err(HtsError::NotFound(format!("CodeSystem not found: {url}")));
    }

    let chosen = match version {
        Some(ver)
            if ver.contains(".x") || ver == "x" || super::code_system_version_is_short(ver) =>
        {
            // Project to (id, version) for the shared matcher then re-attach name.
            let id_ver: Vec<(String, Option<String>)> = candidates
                .iter()
                .map(|(id, _, v)| (id.clone(), v.clone()))
                .collect();
            let (matched_id, _) = super::code_system_select_version_match(&id_ver, ver)
                .ok_or_else(|| {
                    HtsError::NotFound(format!("CodeSystem not found: {url} (version {ver})"))
                })?;
            candidates
                .into_iter()
                .find(|(id, _, _)| id == &matched_id)
                .expect("matched id was sourced from candidates")
        }
        Some(ver) => candidates
            .iter()
            .find(|(_, _, v)| v.as_deref() == Some(ver))
            .cloned()
            .ok_or_else(|| {
                HtsError::NotFound(format!("CodeSystem not found: {url} (version {ver})"))
            })?,
        None => candidates.into_iter().next().expect("non-empty checked"),
    };
    Ok(chosen)
}

/// Fetch every (id, name, version) row for `url`, sorted with the highest
/// version first so `None`-version requests default to the newest revision.
fn fetch_versions(
    conn: &rusqlite::Connection,
    url: &str,
    date: Option<&str>,
) -> Result<Vec<(String, String, Option<String>)>, HtsError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, COALESCE(name, url), version \
             FROM code_systems \
             WHERE url = ?1 \
               AND (?2 IS NULL OR json_extract(resource_json, '$.date') <= ?2) \
             ORDER BY COALESCE(version, '') DESC",
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let rows = stmt
        .query_map(rusqlite::params![url, date], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| HtsError::StorageError(e.to_string()))
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

/// Synthesise hierarchy- and status-derived properties for `$lookup`.
///
/// FHIR defines several "well-known" concept properties whose values are not
/// stored in `concept_properties` directly but are inferred from other tables:
///
/// - `parent` / `child` — derived from `concept_hierarchy`. Each row carries
///   the parent/child code in `value` and the parent/child display in
///   `description`.
/// - `inactive` — boolean derived from a `status` property in the inactive
///   set (retired/deprecated/withdrawn/inactive). Skipped when the concept
///   already has an explicitly-stored `inactive` property to avoid duplicates.
///
/// Returned properties carry `description` populated from the related
/// concept's display so the response includes human-readable context.
fn fetch_synthesised_properties(
    conn: &rusqlite::Connection,
    system_id: &str,
    code: &str,
    stored: &[PropertyValue],
) -> Result<Vec<PropertyValue>, HtsError> {
    let mut out = Vec::new();

    // Parents — synthesised from concept_hierarchy. Skip when the concept
    // already carries explicit `parent` properties (the bundle importer
    // mirrors `parent` properties into concept_hierarchy, so synthesising
    // here would duplicate every stored parent edge).
    let stored_parent_codes: std::collections::HashSet<&str> = stored
        .iter()
        .filter(|p| p.code == "parent")
        .map(|p| p.value.as_str())
        .collect();
    {
        let mut stmt = conn
            .prepare_cached(
                "SELECT h.parent_code, c.display
                 FROM concept_hierarchy h
                 LEFT JOIN concepts c
                        ON c.system_id = h.system_id AND c.code = h.parent_code
                 WHERE h.system_id = ?1 AND h.child_code = ?2
                 ORDER BY h.parent_code",
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![system_id, code], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        for r in rows {
            let (parent_code, parent_display) =
                r.map_err(|e| HtsError::StorageError(e.to_string()))?;
            if stored_parent_codes.contains(parent_code.as_str()) {
                continue;
            }
            out.push(PropertyValue {
                code: "parent".into(),
                value_type: "code".into(),
                value: parent_code,
                description: parent_display,
            });
        }
    }

    // Children — `concept_hierarchy.child_code WHERE parent_code = code`.
    {
        let mut stmt = conn
            .prepare_cached(
                "SELECT h.child_code, c.display
                 FROM concept_hierarchy h
                 LEFT JOIN concepts c
                        ON c.system_id = h.system_id AND c.code = h.child_code
                 WHERE h.system_id = ?1 AND h.parent_code = ?2
                 ORDER BY h.child_code",
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![system_id, code], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        for r in rows {
            let (child_code, child_display) =
                r.map_err(|e| HtsError::StorageError(e.to_string()))?;
            out.push(PropertyValue {
                code: "child".into(),
                value_type: "code".into(),
                value: child_code,
                description: child_display,
            });
        }
    }

    // Inactive flag — synthesise from the `status` property when the concept
    // doesn't already carry an explicit `inactive` row. `inactive=true` when
    // status is in the FHIR inactive set, otherwise `false` (so the response
    // always communicates the active/inactive state).
    if !stored.iter().any(|p| p.code == "inactive") {
        let inactive: bool = conn
            .query_row(
                "SELECT EXISTS (
                     SELECT 1 FROM concept_properties cp
                     JOIN concepts c ON c.id = cp.concept_id
                     WHERE c.system_id = ?1
                       AND c.code = ?2
                       AND cp.property = 'status'
                       AND cp.value IN ('retired', 'deprecated', 'withdrawn', 'inactive')
                 )",
                rusqlite::params![system_id, code],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        out.push(PropertyValue {
            code: "inactive".into(),
            value_type: "boolean".into(),
            value: inactive.to_string(),
            description: None,
        });
    }

    Ok(out)
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
            source: None,
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

    // ── Multi-version resolution ──────────────────────────────────────────────

    /// Insert two versions of the same canonical URL with different concept
    /// displays so we can assert which version got picked.
    fn seed_two_versions(b: &SqliteTerminologyBackend) {
        let conn = b.pool().get().unwrap();
        conn.execute_batch(
            "INSERT INTO code_systems
                 (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('mv|1.0.0', 'http://example.org/mv', '1.0.0', 'MV',
                     'active', 'complete', '2024-01-01', '2024-01-01'),
                    ('mv|1.2.0', 'http://example.org/mv', '1.2.0', 'MV',
                     'active', 'complete', '2024-01-02', '2024-01-02');

             INSERT INTO concepts (id, system_id, code, display)
             VALUES (300, 'mv|1.0.0', 'code1', 'Display 1 (1.0)'),
                    (301, 'mv|1.2.0', 'code1', 'Display 1 (1.2)');",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn lookup_without_version_picks_latest() {
        let b = backend();
        seed_two_versions(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/mv".into(),
                    code: "code1".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.version.as_deref(), Some("1.2.0"));
        assert_eq!(resp.display.as_deref(), Some("Display 1 (1.2)"));
    }

    #[tokio::test]
    async fn lookup_with_exact_version_targets_that_row() {
        let b = backend();
        seed_two_versions(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/mv".into(),
                    code: "code1".into(),
                    version: Some("1.0.0".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.version.as_deref(), Some("1.0.0"));
        assert_eq!(resp.display.as_deref(), Some("Display 1 (1.0)"));
    }

    #[tokio::test]
    async fn lookup_with_partial_wildcard_picks_highest_match() {
        let b = backend();
        seed_two_versions(&b);

        // `1.x.x` matches both 1.0.0 and 1.2.0; the higher one wins.
        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/mv".into(),
                    code: "code1".into(),
                    version: Some("1.x.x".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.version.as_deref(), Some("1.2.0"));
    }

    #[tokio::test]
    async fn lookup_with_short_version_prefix_matches_any_in_family() {
        let b = backend();
        seed_two_versions(&b);

        // Bare numeric prefix `1` should match any 1.x.x version.
        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/mv".into(),
                    code: "code1".into(),
                    version: Some("1".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(resp.version.as_deref(), Some("1.2.0"));
    }

    #[tokio::test]
    async fn lookup_with_unknown_version_returns_not_found() {
        let b = backend();
        seed_two_versions(&b);

        let err = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/mv".into(),
                    code: "code1".into(),
                    version: Some("9.9.9".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HtsError::NotFound(_)));
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

    // ── Synthesised properties (parent / child / inactive / definition) ───────

    /// Seed a three-concept hierarchy used by the synthesis tests below.
    ///
    ///     PARENT
    ///       └── MIDDLE  (status=retired → inactive)
    ///             ├── CHILD_A
    ///             └── CHILD_B
    fn seed_synth(b: &SqliteTerminologyBackend) {
        let conn = b.pool().get().unwrap();
        conn.execute_batch(
            "INSERT INTO code_systems
                 (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('cs-syn', 'http://example.org/syn', '1.0', 'Synth CS',
                     'active', 'complete', '2024-01-01', '2024-01-01');

             INSERT INTO concepts (id, system_id, code, display, definition)
             VALUES (200, 'cs-syn', 'PARENT',  'Parent display', NULL),
                    (201, 'cs-syn', 'MIDDLE',  'Middle display', 'Middle defn'),
                    (202, 'cs-syn', 'CHILD_A', 'Child A display', NULL),
                    (203, 'cs-syn', 'CHILD_B', 'Child B display', NULL);

             INSERT INTO concept_hierarchy (system_id, parent_code, child_code)
             VALUES ('cs-syn', 'PARENT', 'MIDDLE'),
                    ('cs-syn', 'MIDDLE', 'CHILD_A'),
                    ('cs-syn', 'MIDDLE', 'CHILD_B');

             INSERT INTO concept_properties (concept_id, property, value_type, value)
             VALUES (201, 'status', 'code', 'retired');",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn lookup_synthesises_parent_and_child_properties() {
        let b = backend();
        seed_synth(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/syn".into(),
                    code: "MIDDLE".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let parents: Vec<_> = resp
            .properties
            .iter()
            .filter(|p| p.code == "parent")
            .collect();
        assert_eq!(parents.len(), 1);
        assert_eq!(parents[0].value, "PARENT");
        assert_eq!(parents[0].description.as_deref(), Some("Parent display"));

        let children: Vec<_> = resp
            .properties
            .iter()
            .filter(|p| p.code == "child")
            .collect();
        assert_eq!(children.len(), 2);
        // Children are ORDER BY child_code → CHILD_A then CHILD_B.
        assert_eq!(children[0].value, "CHILD_A");
        assert_eq!(children[0].description.as_deref(), Some("Child A display"));
        assert_eq!(children[1].value, "CHILD_B");
    }

    #[tokio::test]
    async fn lookup_synthesises_inactive_from_status_property() {
        let b = backend();
        seed_synth(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/syn".into(),
                    code: "MIDDLE".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // status=retired → inactive=true, surfaced even though concept_properties
        // has no explicit `inactive` row.
        let inactive: Vec<_> = resp
            .properties
            .iter()
            .filter(|p| p.code == "inactive")
            .collect();
        assert_eq!(inactive.len(), 1);
        assert_eq!(inactive[0].value, "true");
        assert_eq!(inactive[0].value_type, "boolean");
    }

    #[tokio::test]
    async fn lookup_synthesises_inactive_false_when_no_status() {
        let b = backend();
        seed_synth(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/syn".into(),
                    code: "PARENT".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let inactive = resp
            .properties
            .iter()
            .find(|p| p.code == "inactive")
            .unwrap();
        assert_eq!(inactive.value, "false");
    }

    #[tokio::test]
    async fn lookup_does_not_duplicate_explicit_inactive() {
        let b = backend();
        let conn = b.pool().get().unwrap();
        conn.execute_batch(
            "INSERT INTO code_systems
                 (id, url, version, name, status, content, created_at, updated_at)
             VALUES ('cs-i', 'http://example.org/i', '1.0', 'I CS',
                     'active', 'complete', '2024-01-01', '2024-01-01');
             INSERT INTO concepts (id, system_id, code, display)
             VALUES (300, 'cs-i', 'X', 'X display');
             INSERT INTO concept_properties (concept_id, property, value_type, value)
             VALUES (300, 'inactive', 'boolean', 'false');",
        )
        .unwrap();
        drop(conn);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/i".into(),
                    code: "X".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Exactly one inactive property — synthesis is skipped because the
        // concept already has an explicit `inactive` row.
        let inactive: Vec<_> = resp
            .properties
            .iter()
            .filter(|p| p.code == "inactive")
            .collect();
        assert_eq!(inactive.len(), 1);
        assert_eq!(inactive[0].value, "false");
    }

    #[tokio::test]
    async fn lookup_returns_definition_field() {
        let b = backend();
        seed_synth(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/syn".into(),
                    code: "MIDDLE".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.definition.as_deref(), Some("Middle defn"));
    }

    #[tokio::test]
    async fn lookup_property_filter_includes_synthesised_codes() {
        let b = backend();
        seed_synth(&b);

        // Asking only for `parent` should return just the synthesised parent —
        // no children, no inactive, even though those would surface under `*`.
        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/syn".into(),
                    code: "MIDDLE".into(),
                    properties: vec!["parent".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.properties.len(), 1);
        assert_eq!(resp.properties[0].code, "parent");
        assert_eq!(resp.properties[0].value, "PARENT");
    }

    #[tokio::test]
    async fn lookup_wildcard_includes_synthesised_and_stored() {
        let b = backend();
        seed_synth(&b);

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/syn".into(),
                    code: "MIDDLE".into(),
                    properties: vec!["*".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Stored: status. Synthesised: parent, child x2, inactive.
        let codes: Vec<_> = resp.properties.iter().map(|p| p.code.as_str()).collect();
        assert!(codes.contains(&"status"));
        assert!(codes.contains(&"parent"));
        assert!(codes.contains(&"child"));
        assert!(codes.contains(&"inactive"));
    }
}
