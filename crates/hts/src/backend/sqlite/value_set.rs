//! SQLite implementation of [`ValueSetOperations`].
//!
//! Phase 7: `$expand` and `ValueSet/$validate-code`.
//!
//! **Expansion strategy:**
//! - Lazy: expansion is computed on the first `$expand` call and cached in
//!   `value_set_expansions`.
//! - `compose.include[]` supports both explicit code lists and full-system
//!   includes (when no `concept` array is provided).
//! - `compose.exclude[]` removes specific codes after include is resolved.
//! - Pagination (`count` + `offset`) is applied in-memory after expansion.

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;

use crate::error::HtsError;
use crate::traits::ValueSetOperations;
use crate::types::{
    ExpandRequest, ExpandResponse, ExpansionContains, ValidateCodeRequest, ValidateCodeResponse,
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

            let (vs_id, compose_json) = resolve_value_set(&conn, &url)?;

            // Check expansion cache first.
            let mut all_codes = fetch_cache(&conn, &vs_id)?;

            if all_codes.is_empty() {
                // Cache miss: compute and populate.
                all_codes = compute_expansion(&conn, compose_json.as_deref())?;
                populate_cache(&conn, &vs_id, &all_codes)?;
            }

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
            let (vs_id, compose_json) = match resolve_value_set(&conn, &url) {
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
                    Ok(ValidateCodeResponse {
                        result: true,
                        message,
                        display: concept.display.clone(),
                    })
                }
            }
        })
        .await
        .map_err(|e| HtsError::Internal(format!("Blocking task error: {e}")))?
    }
}

// ── Private helpers ────────────────────────────────────────────────────────────

/// Resolve a value set by canonical URL, returning `(id, compose_json)`.
///
/// Returns [`HtsError::NotFound`] when the URL is not in the `value_sets` table.
fn resolve_value_set(conn: &Connection, url: &str) -> Result<(String, Option<String>), HtsError> {
    conn.query_row(
        "SELECT id, compose_json FROM value_sets WHERE url = ?1",
        [url],
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

        if let Some(explicit_codes) = inc["concept"].as_array() {
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
            // No explicit codes: include ALL concepts from the referenced system.
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
    use crate::backend::sqlite::SqliteTerminologyBackend;
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
                    value_set: None,
                    filter: None,
                    count: None,
                    offset: None,
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
                    value_set: None,
                    filter: None,
                    count: None,
                    offset: None,
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
                    value_set: None,
                    filter: None,
                    count: Some(1),
                    offset: Some(0),
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
                    value_set: None,
                    filter: None,
                    count: Some(1),
                    offset: Some(1),
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
                    value_set: None,
                    filter: Some("Concept A".into()),
                    count: None,
                    offset: None,
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
            value_set: None,
            filter: None,
            count: None,
            offset: None,
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
                    value_set: None,
                    filter: None,
                    count: None,
                    offset: None,
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
                    url: None,
                    value_set: None,
                    filter: None,
                    count: None,
                    offset: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, HtsError::InvalidRequest(_)));
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
                    system: None,
                    code: "A".into(),
                    version: None,
                    display: None,
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
                    system: None,
                    code: "C".into(), // C is in CodeSystem but NOT in the ValueSet
                    version: None,
                    display: None,
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
                    system: None,
                    code: "A".into(),
                    version: None,
                    display: None,
                },
            )
            .await
            .unwrap();

        assert!(!resp.result);
    }

    // ── $validate-code: display mismatch returns true with message ─────────────

    #[tokio::test]
    async fn validate_code_display_mismatch_is_true_with_message() {
        let b = backend();
        b.import_bundle(&ctx(), bundle_with_explicit_codes().as_bytes())
            .await
            .unwrap();

        let resp = b
            .validate_code(
                &ctx(),
                ValidateCodeRequest {
                    url: Some("http://example.org/vs".into()),
                    system: None,
                    code: "A".into(),
                    version: None,
                    display: Some("Wrong Display".into()),
                },
            )
            .await
            .unwrap();

        assert!(resp.result, "code is valid even with a display mismatch");
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
                    system: None,
                    code: "A".into(),
                    version: None,
                    display: Some("Concept A".into()),
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
                    value_set: None,
                    filter: None,
                    count: None,
                    offset: None,
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
                    value_set: None,
                    filter: None,
                    count: None,
                    offset: None,
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
                    system: None,
                    code: "A".into(),
                    version: None,
                    display: None,
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
                    system: None,
                    code: "C".into(),
                    version: None,
                    display: None,
                },
            )
            .await
            .unwrap();
        assert!(!v_out.result);
    }
}
