//! PostgreSQL implementation of [`CodeSystemOperations`].

#![cfg(feature = "postgres")]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;

use crate::error::HtsError;
use crate::traits::{CodeSystemOperations, ConceptDesignation, ConceptExpansionFlags};
use crate::types::{
    DesignationValue, LookupRequest, LookupResponse, PropertyValue, ResourceSearchQuery,
    SubsumesRequest, SubsumesResponse, SubsumptionOutcome, ValidateCodeRequest,
    ValidateCodeResponse,
};

use super::PostgresTerminologyBackend;

#[async_trait]
impl CodeSystemOperations for PostgresTerminologyBackend {
    async fn lookup(
        &self,
        _ctx: &TenantContext,
        req: LookupRequest,
    ) -> Result<LookupResponse, HtsError> {
        if req.expression.is_some() {
            return Err(HtsError::NotSupported(
                "SNOMED post-coordination expressions are not supported in the PostgreSQL backend"
                    .into(),
            ));
        }

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let (system_id, cs_name, cs_version) = resolve_code_system(
            &client,
            &req.system,
            req.version.as_deref(),
            req.date.as_deref(),
        )
        .await?;

        let (concept_id, display, definition) =
            find_concept(&client, &system_id, &req.code).await?;

        let stored_props = fetch_properties(&client, concept_id).await?;
        // Per FHIR spec, property="*" is the wildcard meaning "include
        // every property the concept has".
        let want_all = req.properties.is_empty() || req.properties.iter().any(|p| p == "*");
        let synth_props =
            fetch_synthesised_properties(&client, &system_id, &req.code, &stored_props).await?;
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

        let all_designations = fetch_designations(&client, concept_id).await?;

        let display = if let Some(lang) = req.display_language.as_deref() {
            all_designations
                .iter()
                .find(|d| d.language.as_deref() == Some(lang))
                .map(|d| d.value.clone())
                .or(display)
        } else {
            display
        };

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
    }

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

        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let system_id = match resolve_code_system(
            &client,
            &system,
            req.version.as_deref(),
            req.date.as_deref(),
        )
        .await
        {
            Ok((id, _, _)) => id,
            Err(HtsError::NotFound(_)) => {
                return Ok(ValidateCodeResponse {
                    result: false,
                    message: Some(format!("Unknown code system: {system}")),
                    display: None,
                    system: None,
                    inactive: None,
                    issues: vec![],
                    caused_by_unknown_system: None,
                });
            }
            Err(e) => return Err(e),
        };

        let display = match find_concept(&client, &system_id, &req.code).await {
            Ok((_, display, _)) => display,
            Err(HtsError::NotFound(_)) => {
                return Ok(ValidateCodeResponse {
                    result: false,
                    message: Some(format!("Unknown code: {}", req.code)),
                    display: None,
                    system: None,
                    inactive: None,
                    issues: vec![],
                    caused_by_unknown_system: None,
                });
            }
            Err(e) => return Err(e),
        };

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
            system: None,
            inactive: None,
            issues: vec![],
            caused_by_unknown_system: None,
        })
    }

    async fn subsumes(
        &self,
        _ctx: &TenantContext,
        req: SubsumesRequest,
    ) -> Result<SubsumesResponse, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let (system_id, _, _) =
            resolve_code_system(&client, &req.system, req.version.as_deref(), None).await?;

        find_concept(&client, &system_id, &req.code_a).await?;
        find_concept(&client, &system_id, &req.code_b).await?;

        if req.code_a == req.code_b {
            return Ok(SubsumesResponse {
                outcome: SubsumptionOutcome::Equivalent,
            });
        }

        if check_ancestor(&client, &system_id, &req.code_a, &req.code_b).await? {
            return Ok(SubsumesResponse {
                outcome: SubsumptionOutcome::Subsumes,
            });
        }

        if check_ancestor(&client, &system_id, &req.code_b, &req.code_a).await? {
            return Ok(SubsumesResponse {
                outcome: SubsumptionOutcome::SubsumedBy,
            });
        }

        Ok(SubsumesResponse {
            outcome: SubsumptionOutcome::NotSubsumed,
        })
    }

    async fn code_system_version_for_url(
        &self,
        _ctx: &TenantContext,
        url: &str,
    ) -> Result<Option<String>, HtsError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let row = client
            .query_opt(
                "SELECT version FROM code_systems WHERE url = $1 LIMIT 1",
                &[&url],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        Ok(row.and_then(|r| r.get::<_, Option<String>>(0)))
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
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let rows = client
            .query(
                "SELECT c.code, cd.language, cd.use_system, cd.use_code, cd.value
                 FROM concept_designations cd
                 JOIN concepts c ON c.id = cd.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = $1
                   AND c.code = ANY($2)",
                &[&system_url, &codes],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let mut out: std::collections::HashMap<String, Vec<ConceptDesignation>> =
            std::collections::HashMap::new();
        for row in rows {
            let code: String = row.get(0);
            out.entry(code).or_default().push(ConceptDesignation {
                language: row.get(1),
                use_system: row.get(2),
                use_code: row.get(3),
                value: row.get(4),
                source: None,
            });
        }
        Ok(out)
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
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
        let rows = client
            .query(
                "SELECT c.code, cp.property, cp.value
                 FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = $1
                   AND c.code = ANY($2)
                   AND cp.property = ANY($3)",
                &[&system_url, &codes, &properties],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let mut out: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for row in rows {
            let code: String = row.get(0);
            let prop: String = row.get(1);
            let value: String = row.get(2);
            out.entry(code).or_default().push((prop, value));
        }
        Ok(out)
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
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let rows = client
            .query(
                "SELECT c.code, cp.property, cp.value
                 FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 JOIN code_systems s ON s.id = c.system_id
                 WHERE s.url = $1
                   AND c.code = ANY($2)
                   AND cp.property IN ('notSelectable', 'status')",
                &[&system_url, &codes],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let mut out: std::collections::HashMap<String, ConceptExpansionFlags> =
            std::collections::HashMap::new();
        for row in rows {
            let code: String = row.get(0);
            let property: String = row.get(1);
            let value: String = row.get(2);
            let flags = out.entry(code).or_default();
            match property.as_str() {
                "notSelectable" if value == "true" => flags.is_abstract = true,
                "status"
                    if matches!(
                        value.as_str(),
                        "retired" | "deprecated" | "withdrawn" | "inactive"
                    ) =>
                {
                    flags.inactive = true;
                }
                _ => {}
            }
        }
        Ok(out)
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
                 FROM code_systems
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
                    "CodeSystem",
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

// ── Private DB helpers ─────────────────────────────────────────────────────────

/// Resolve a code system by URL, optional version, and optional date.
///
/// Returns `(id, name_or_url, version)`.
///
/// Mirrors the SQLite implementation: an unspecified version defaults to the
/// most recent (textual COALESCE-DESC), an explicit version with `.x` segments
/// (or a bare numeric prefix like `"1"`) matches the highest version that
/// shares the literal segments, and an exact version requires an exact match.
async fn resolve_code_system(
    client: &tokio_postgres::Client,
    url: &str,
    version: Option<&str>,
    date: Option<&str>,
) -> Result<(String, String, Option<String>), HtsError> {
    let rows = client
        .query(
            "SELECT id, COALESCE(name, url), version
             FROM code_systems
             WHERE url = $1
               AND ($2::text IS NULL OR (resource_json->>'date') <= $2)
             ORDER BY COALESCE(version, '') DESC",
            &[&url, &date],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    if rows.is_empty() {
        return Err(HtsError::NotFound(format!("CodeSystem not found: {url}")));
    }
    let candidates: Vec<(String, String, Option<String>)> = rows
        .into_iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();

    match version {
        Some(ver) if ver.contains(".x") || ver == "x" || is_short_version(ver) => {
            select_best_version_match(&candidates, ver).ok_or_else(|| {
                HtsError::NotFound(format!("CodeSystem not found: {url} (version {ver})"))
            })
        }
        Some(ver) => candidates
            .into_iter()
            .find(|(_, _, v)| v.as_deref() == Some(ver))
            .ok_or_else(|| {
                HtsError::NotFound(format!("CodeSystem not found: {url} (version {ver})"))
            }),
        None => Ok(candidates.into_iter().next().expect("non-empty checked")),
    }
}

fn is_short_version(ver: &str) -> bool {
    !ver.contains('.') && ver.chars().all(|c| c.is_ascii_digit())
}

fn select_best_version_match(
    candidates: &[(String, String, Option<String>)],
    pattern: &str,
) -> Option<(String, String, Option<String>)> {
    let pattern_segments: Vec<&str> = pattern.split('.').collect();
    candidates
        .iter()
        .filter(|(_, _, v)| match v {
            Some(actual) => version_matches(actual, &pattern_segments),
            None => false,
        })
        .max_by(|a, b| a.2.cmp(&b.2))
        .cloned()
}

fn version_matches(actual: &str, pattern_segments: &[&str]) -> bool {
    let actual_segments: Vec<&str> = actual.split('.').collect();
    if pattern_segments.len() > actual_segments.len() {
        return false;
    }
    pattern_segments
        .iter()
        .zip(actual_segments.iter())
        .all(|(p, a)| *p == "x" || *p == *a)
}

/// Look up a concept row by `(system_id, code)`.
///
/// Returns `(concept_id, display, definition)`.
async fn find_concept(
    client: &tokio_postgres::Client,
    system_id: &str,
    code: &str,
) -> Result<(i64, Option<String>, Option<String>), HtsError> {
    let rows = client
        .query(
            "SELECT id, display, definition FROM concepts
             WHERE system_id = $1 AND code = $2",
            &[&system_id, &code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| HtsError::NotFound(format!("Concept not found: {code}")))?;

    Ok((row.get(0), row.get(1), row.get(2)))
}

/// Fetch all properties for a concept.
async fn fetch_properties(
    client: &tokio_postgres::Client,
    concept_id: i64,
) -> Result<Vec<PropertyValue>, HtsError> {
    let rows = client
        .query(
            "SELECT property, value_type, value
             FROM concept_properties WHERE concept_id = $1 ORDER BY property",
            &[&concept_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| PropertyValue {
            code: row.get(0),
            value_type: row.get(1),
            value: row.get(2),
            description: None,
        })
        .collect())
}

/// Synthesise hierarchy- and status-derived properties for `$lookup`.
///
/// Mirrors the SQLite backend implementation — see
/// [`super::super::sqlite::code_system::fetch_synthesised_properties`] for
/// rationale.
async fn fetch_synthesised_properties(
    client: &tokio_postgres::Client,
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
    let parent_rows = client
        .query(
            "SELECT h.parent_code, c.display
             FROM concept_hierarchy h
             LEFT JOIN concepts c
                    ON c.system_id = h.system_id AND c.code = h.parent_code
             WHERE h.system_id = $1 AND h.child_code = $2
             ORDER BY h.parent_code",
            &[&system_id, &code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    for row in parent_rows {
        let parent_code: String = row.get(0);
        if stored_parent_codes.contains(parent_code.as_str()) {
            continue;
        }
        out.push(PropertyValue {
            code: "parent".into(),
            value_type: "code".into(),
            value: parent_code,
            description: row.get(1),
        });
    }

    // Children.
    let child_rows = client
        .query(
            "SELECT h.child_code, c.display
             FROM concept_hierarchy h
             LEFT JOIN concepts c
                    ON c.system_id = h.system_id AND c.code = h.child_code
             WHERE h.system_id = $1 AND h.parent_code = $2
             ORDER BY h.child_code",
            &[&system_id, &code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    for row in child_rows {
        out.push(PropertyValue {
            code: "child".into(),
            value_type: "code".into(),
            value: row.get(0),
            description: row.get(1),
        });
    }

    // Inactive flag (only when not already stored explicitly).
    if !stored.iter().any(|p| p.code == "inactive") {
        let row = client
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM concept_properties cp
                     JOIN concepts c ON c.id = cp.concept_id
                     WHERE c.system_id = $1
                       AND c.code = $2
                       AND cp.property = 'status'
                       AND cp.value IN ('retired', 'deprecated', 'withdrawn', 'inactive')
                 )",
                &[&system_id, &code],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        let inactive: bool = row.get(0);
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
async fn fetch_designations(
    client: &tokio_postgres::Client,
    concept_id: i64,
) -> Result<Vec<DesignationValue>, HtsError> {
    let rows = client
        .query(
            "SELECT language, use_system, use_code, value
             FROM concept_designations WHERE concept_id = $1",
            &[&concept_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| DesignationValue {
            language: row.get(0),
            use_system: row.get(1),
            use_code: row.get(2),
            value: row.get(3),
            source: None,
        })
        .collect())
}

/// Return `true` if `ancestor_code` is a (possibly indirect) ancestor of
/// `descendant_code` within the given code system.
///
/// Uses a recursive CTE to walk up the parent chain.
pub(super) async fn check_ancestor(
    client: &tokio_postgres::Client,
    system_id: &str,
    ancestor_code: &str,
    descendant_code: &str,
) -> Result<bool, HtsError> {
    let rows = client
        .query(
            "WITH RECURSIVE ancestors(code) AS (
                 SELECT parent_code
                 FROM   concept_hierarchy
                 WHERE  system_id = $1 AND child_code = $3
                 UNION
                 SELECT h.parent_code
                 FROM   concept_hierarchy h
                 INNER JOIN ancestors a ON a.code = h.child_code
                 WHERE  h.system_id = $1
             )
             SELECT 1 FROM ancestors WHERE code = $2 LIMIT 1",
            &[&system_id, &ancestor_code, &descendant_code],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(!rows.is_empty())
}

/// Build a minimal synthetic FHIR resource JSON when `resource_json` is absent.
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
