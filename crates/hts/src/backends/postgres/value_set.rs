//! PostgreSQL implementation of [`ValueSetOperations`].

#![cfg(feature = "postgres")]

use async_trait::async_trait;
use helios_persistence::tenant::TenantContext;
use std::collections::{HashMap, HashSet};

use crate::error::HtsError;
use crate::traits::ValueSetOperations;
use crate::types::{
    ExpandRequest, ExpandResponse, ExpansionContains, ResourceSearchQuery, ValidateCodeRequest,
    ValidateCodeResponse,
};

use super::PostgresTerminologyBackend;
use super::code_system::build_synthetic_resource;

#[async_trait]
impl ValueSetOperations for PostgresTerminologyBackend {
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

        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let all_codes = match resolve_value_set(&client, &url, req.date.as_deref()).await {
            Ok((vs_id, compose_json)) => {
                let cached = fetch_cache(&client, &vs_id).await?;
                if cached.is_empty() {
                    let codes = compute_expansion(&client, compose_json.as_deref()).await?;
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
                    populate_cache(&mut client, &vs_id, &codes).await?;
                    codes
                } else {
                    cached
                }
            }
            Err(HtsError::NotFound(_)) => {
                let cs_url = find_cs_for_implicit_vs(&client, &url, req.date.as_deref()).await?;
                let compose = serde_json::json!({
                    "include": [{ "system": cs_url }]
                })
                .to_string();
                let codes = compute_expansion(&client, Some(&compose)).await?;
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
                codes
            }
            Err(e) => return Err(e),
        };

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

        if req.hierarchical == Some(true) {
            let total = filtered.len() as u32;
            let tree = build_hierarchical_expansion(&client, filtered).await?;
            return Ok(ExpandResponse {
                total: Some(total),
                offset: None,
                contains: tree,
                warnings: vec![],
            });
        }

        let total = filtered.len() as u32;
        let offset = req.offset.unwrap_or(0) as usize;
        let count = req.count.map(|c| c as usize).unwrap_or(usize::MAX);

        let page: Vec<ExpansionContains> = filtered.into_iter().skip(offset).take(count).collect();

        Ok(ExpandResponse {
            total: Some(total),
            offset: req.offset,
            contains: page,
            warnings: vec![],
        })
    }

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

        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;

        let (vs_id, compose_json) =
            match resolve_value_set(&client, &url, req.date.as_deref()).await {
                Ok(vs) => vs,
                Err(HtsError::NotFound(_)) => {
                    return Ok(ValidateCodeResponse {
                        result: false,
                        message: Some(format!("Unknown value set: {url}")),
                        display: None,
                        inactive: None,
                    });
                }
                Err(e) => return Err(e),
            };

        let cached = fetch_cache(&client, &vs_id).await?;
        let all_codes = if cached.is_empty() {
            let codes = compute_expansion(&client, compose_json.as_deref()).await?;
            populate_cache(&mut client, &vs_id, &codes).await?;
            codes
        } else {
            cached
        };

        let found = if let Some(system) = req.system.as_deref() {
            all_codes
                .iter()
                .find(|c| c.system == system && c.code == req.code)
        } else {
            all_codes.iter().find(|c| c.code == req.code)
        };

        match found {
            None => {
                let qualified = match req.system.as_deref() {
                    Some(s) => format!("{s}#{}", req.code),
                    None => req.code.clone(),
                };
                Ok(ValidateCodeResponse {
                    result: false,
                    message: Some(format!(
                        "The provided code '{qualified}' was not found in the value set '{url}'"
                    )),
                    display: None,
                    inactive: None,
                })
            }
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
                    result: message.is_none(),
                    message,
                    display: concept.display.clone(),
                    inactive: None,
                })
            }
        }
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
                 FROM value_sets
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
                    "ValueSet",
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

/// Resolve a value set by canonical URL and optional point-in-time date.
///
/// Returns `(id, compose_json)`.
async fn resolve_value_set(
    client: &tokio_postgres::Client,
    url: &str,
    date: Option<&str>,
) -> Result<(String, Option<String>), HtsError> {
    let rows = client
        .query(
            "SELECT id, compose_json FROM value_sets
             WHERE url = $1
               AND ($2::text IS NULL OR (resource_json->>'date') <= $2)",
            &[&url, &date],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let row = rows.into_iter().next().ok_or_else(|| {
        HtsError::NotFound(format!(
            "A definition for the value Set \'{url}\' could not be found"
        ))
    })?;

    Ok((row.get(0), row.get(1)))
}

/// Fetch all cached expansion entries for `vs_id`.
async fn fetch_cache(
    client: &tokio_postgres::Client,
    vs_id: &str,
) -> Result<Vec<ExpansionContains>, HtsError> {
    let rows = client
        .query(
            "SELECT system_url, code, display
             FROM value_set_expansions
             WHERE value_set_id = $1
             ORDER BY system_url, code",
            &[&vs_id],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|row| ExpansionContains {
            system: row.get(0),
            code: row.get(1),
            display: row.get(2),
            is_abstract: None,

            inactive: None,

            designations: vec![],
            contains: vec![],
        })
        .collect())
}

/// Compute an expansion from the raw `compose_json`.
async fn compute_expansion(
    client: &tokio_postgres::Client,
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

        let rows = client
            .query("SELECT id FROM code_systems WHERE url = $1", &[&system_url])
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let system_id: String = match rows.into_iter().next() {
            Some(r) => r.get(0),
            None => {
                tracing::warn!(
                    system_url,
                    "Skipping unknown code system in ValueSet compose"
                );
                continue;
            }
        };

        if let Some(explicit_codes) = inc["concept"].as_array() {
            for entry in explicit_codes {
                let code = match entry["code"].as_str() {
                    Some(c) => c.to_owned(),
                    None => continue,
                };

                let disp_rows = client
                    .query(
                        "SELECT display FROM concepts WHERE system_id = $1 AND code = $2",
                        &[&system_id, &code],
                    )
                    .await
                    .map_err(|e| HtsError::StorageError(e.to_string()))?;

                let display: Option<String> = disp_rows.into_iter().next().and_then(|r| r.get(0));

                included.push(ExpansionContains {
                    system: system_url.to_owned(),
                    code,
                    display,
                    is_abstract: None,

                    inactive: None,

                    designations: vec![],
                    contains: vec![],
                });
            }
        } else {
            let code_rows = client
                .query(
                    "SELECT code, display FROM concepts WHERE system_id = $1 ORDER BY code",
                    &[&system_id],
                )
                .await
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            for row in code_rows {
                included.push(ExpansionContains {
                    system: system_url.to_owned(),
                    code: row.get(0),
                    display: row.get(1),
                    is_abstract: None,

                    inactive: None,

                    designations: vec![],
                    contains: vec![],
                });
            }
        }
    }

    // Apply excludes.
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

/// Find the canonical URL of a CodeSystem whose `valueSet` property equals `vs_url`.
async fn find_cs_for_implicit_vs(
    client: &tokio_postgres::Client,
    vs_url: &str,
    date: Option<&str>,
) -> Result<String, HtsError> {
    let rows = client
        .query(
            "SELECT url FROM code_systems
             WHERE (resource_json->>'valueSet') = $1
               AND ($2::text IS NULL OR (resource_json->>'date') <= $2)",
            &[&vs_url, &date],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    rows.into_iter()
        .next()
        .map(|r| r.get::<_, String>(0))
        .ok_or_else(|| {
            HtsError::NotFound(format!(
                "A definition for the value Set \'{vs_url}\' could not be found"
            ))
        })
}

/// Build a tree-structured expansion from a flat list of concepts.
async fn build_hierarchical_expansion(
    client: &tokio_postgres::Client,
    flat: Vec<ExpansionContains>,
) -> Result<Vec<ExpansionContains>, HtsError> {
    if flat.is_empty() {
        return Ok(flat);
    }

    let items_map: HashMap<(String, String), ExpansionContains> = flat
        .iter()
        .cloned()
        .map(|c| ((c.system.clone(), c.code.clone()), c))
        .collect();

    let expansion_set: HashSet<(String, String)> = flat
        .iter()
        .map(|c| (c.system.clone(), c.code.clone()))
        .collect();

    let system_urls: HashSet<String> = flat.iter().map(|c| c.system.clone()).collect();
    let mut system_id_map: HashMap<String, String> = HashMap::new();
    for sys_url in &system_urls {
        let rows = client
            .query("SELECT id FROM code_systems WHERE url = $1", &[sys_url])
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        if let Some(row) = rows.into_iter().next() {
            system_id_map.insert(sys_url.clone(), row.get(0));
        }
    }

    let mut parent_to_children: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    let mut has_parent: HashSet<(String, String)> = HashSet::new();

    for (sys_url, sys_id) in &system_id_map {
        let edge_rows = client
            .query(
                "SELECT parent_code, child_code FROM concept_hierarchy WHERE system_id = $1",
                &[sys_id],
            )
            .await
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        for row in edge_rows {
            let parent_code: String = row.get(0);
            let child_code: String = row.get(1);
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
/// Uses a transaction so the DELETE+INSERTs are atomic: without it, another
/// connection running `write_value_set` (which also DELETEs expansion rows
/// for the same `value_set_id` inside its own import transaction) can
/// interleave its buffered DELETE with our per-statement autocommits and
/// leave the cache with only the entries we inserted *after* that DELETE.
/// The symptom is a validate-code call reading a partially populated cache
/// (e.g. only the last-inserted code survives).
async fn populate_cache(
    client: &mut tokio_postgres::Client,
    vs_id: &str,
    codes: &[ExpansionContains],
) -> Result<(), HtsError> {
    let tx = client
        .transaction()
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    tx.execute(
        "DELETE FROM value_set_expansions WHERE value_set_id = $1",
        &[&vs_id],
    )
    .await
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    for item in codes {
        tx.execute(
            "INSERT INTO value_set_expansions (value_set_id, system_url, code, display)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
            &[&vs_id, &item.system, &item.code, &item.display],
        )
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(())
}
