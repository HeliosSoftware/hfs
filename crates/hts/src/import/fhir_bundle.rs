//! FHIR Bundle importer.
//!
//! Parses a FHIR Bundle JSON document and imports all contained
//! `CodeSystem`, `ValueSet`, and `ConceptMap` resources into the
//! SQLite terminology store.
//!
//! Import order is always: CodeSystems → ValueSets → ConceptMaps, so that
//! value set compose rules and concept map groups can reference code systems
//! that were inserted in the same bundle.

#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;
#[cfg(feature = "sqlite")]
use rusqlite::Connection;

use serde_json::Value;
use uuid::Uuid;

use crate::error::HtsError;
use crate::import::ImportStats;

// ── Core import logic (synchronous, runs in spawn_blocking) ───────────────────

/// Parse a FHIR Bundle from raw bytes and insert its resources into SQLite.
///
/// Called by `SqliteTerminologyBackend::import_bundle` and the `POST /import`
/// HTTP handler.
#[cfg(feature = "sqlite")]
pub(crate) fn import_bundle_sync(
    pool: &Pool<SqliteConnectionManager>,
    data: &[u8],
) -> Result<ImportStats, HtsError> {
    let bundle: Value = serde_json::from_slice(data)
        .map_err(|e| HtsError::InvalidRequest(format!("Invalid JSON: {e}")))?;

    let resource_type = bundle["resourceType"].as_str().unwrap_or("");
    if resource_type != "Bundle" {
        return Err(HtsError::InvalidRequest(format!(
            "Expected a FHIR Bundle resource, got '{resource_type}'"
        )));
    }

    // Partition entries by resource type, preserving document order within each.
    let mut code_systems: Vec<Value> = vec![];
    let mut value_sets: Vec<Value> = vec![];
    let mut concept_maps: Vec<Value> = vec![];

    let entries = bundle["entry"].as_array().cloned().unwrap_or_default();
    for entry in &entries {
        let resource = &entry["resource"];
        if resource.is_null() {
            continue;
        }
        match resource["resourceType"].as_str() {
            Some("CodeSystem") => code_systems.push(resource.clone()),
            Some("ValueSet") => value_sets.push(resource.clone()),
            Some("ConceptMap") => concept_maps.push(resource.clone()),
            Some(rt) => {
                tracing::debug!(
                    resource_type = rt,
                    "Skipping unsupported resource type in Bundle"
                );
            }
            None => {}
        }
    }

    let conn = pool
        .get()
        .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
    let mut stats = ImportStats::default();

    // ── Import CodeSystems first (ValueSets/ConceptMaps may reference them) ──
    for cs in &code_systems {
        let url = cs["url"].as_str().unwrap_or("unknown");
        if let Err(e) = import_code_system(&conn, cs, &mut stats) {
            stats
                .errors
                .push(format!("CodeSystem '{url}' import failed: {e}"));
        }
    }

    // ── ValueSets ──────────────────────────────────────────────────────────────
    for vs in &value_sets {
        let url = vs["url"].as_str().unwrap_or("unknown");
        if let Err(e) = import_value_set(&conn, vs, &mut stats) {
            stats
                .errors
                .push(format!("ValueSet '{url}' import failed: {e}"));
        }
    }

    // ── ConceptMaps ────────────────────────────────────────────────────────────
    for cm in &concept_maps {
        let url = cm["url"].as_str().unwrap_or("unknown");
        if let Err(e) = import_concept_map(&conn, cm, &mut stats) {
            stats
                .errors
                .push(format!("ConceptMap '{url}' import failed: {e}"));
        }
    }

    Ok(stats)
}

// ── CodeSystem ─────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
pub(crate) fn import_code_system(
    conn: &Connection,
    cs: &Value,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let id = cs["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let url = cs["url"]
        .as_str()
        .ok_or_else(|| HtsError::InvalidRequest("CodeSystem.url is required".into()))?;

    let version = cs["version"].as_str();
    let name = cs["name"].as_str();
    let title = cs["title"].as_str();
    let status = cs["status"].as_str().unwrap_or("active");
    let content = cs["content"].as_str().unwrap_or("complete");
    let resource_json = serde_json::to_string(cs).ok();
    let now = utc_now();

    conn.execute(
        "INSERT OR REPLACE INTO code_systems
         (id, url, version, name, title, status, content, resource_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        rusqlite::params![
            id,
            url,
            version,
            name,
            title,
            status,
            content,
            resource_json,
            now
        ],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Import all top-level concepts (will recurse into children).
    if let Some(concepts) = cs["concept"].as_array() {
        import_concepts(conn, &id, concepts, None, stats)?;
    }

    stats.code_systems += 1;
    Ok(())
}

/// Recursively import concepts, tracking the parent code for hierarchy edges.
///
/// Hierarchy is built from two sources:
/// 1. **Nesting** — a concept listed under another concept's `concept` array is
///    a direct child of the enclosing concept.
/// 2. **Parent property** — a concept carrying `property[{code:"parent", valueCode:"<X>"}]`
///    is a direct child of `X`.
#[cfg(feature = "sqlite")]
fn import_concepts(
    conn: &Connection,
    system_id: &str,
    concepts: &[Value],
    parent_code: Option<&str>,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    for concept_val in concepts {
        let code = match concept_val["code"].as_str() {
            Some(c) => c.to_owned(),
            None => {
                stats.errors.push(format!(
                    "Concept in system '{system_id}' is missing the 'code' field — skipped"
                ));
                continue;
            }
        };

        let display = concept_val["display"].as_str().map(str::to_owned);
        let definition = concept_val["definition"].as_str().map(str::to_owned);

        conn.execute(
            "INSERT OR REPLACE INTO concepts (system_id, code, display, definition)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![system_id, code, display, definition],
        )
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

        let concept_id = conn.last_insert_rowid();
        stats.concepts += 1;

        // Hierarchy from nesting: enclosing concept is parent.
        if let Some(parent) = parent_code {
            insert_hierarchy(conn, system_id, parent, &code)?;
        }

        // Properties (may also contribute parent-child hierarchy rows).
        if let Some(props) = concept_val["property"].as_array() {
            for prop in props {
                import_concept_property(conn, concept_id, prop, system_id, &code)?;
            }
        }

        // Designations (alternate names / translations).
        if let Some(desigs) = concept_val["designation"].as_array() {
            for desig in desigs {
                import_designation(conn, concept_id, desig)?;
            }
        }

        // Recurse: nested child concepts.
        if let Some(children) = concept_val["concept"].as_array() {
            import_concepts(conn, system_id, children, Some(&code), stats)?;
        }
    }

    Ok(())
}

/// Insert one concept property row and, if the property code is `"parent"`,
/// also add a `concept_hierarchy` edge.
#[cfg(feature = "sqlite")]
fn import_concept_property(
    conn: &Connection,
    concept_id: i64,
    prop: &Value,
    system_id: &str,
    concept_code: &str,
) -> Result<(), HtsError> {
    let property_code = match prop["code"].as_str() {
        Some(c) => c.to_owned(),
        None => return Ok(()), // malformed property — skip silently
    };

    let (value_type, value) = extract_property_value(prop);
    if value.is_empty() {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO concept_properties (concept_id, property, value_type, value)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![concept_id, property_code, value_type, value],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // FHIR convention: property code "parent" with valueCode = parent code.
    if property_code == "parent" && value_type == "code" {
        insert_hierarchy(conn, system_id, &value, concept_code)?;
    }

    Ok(())
}

/// Detect the FHIR property value type from the `valueXxx` fields present in
/// the JSON object and return `(value_type, serialised_value)`.
#[cfg(feature = "sqlite")]
fn extract_property_value(prop: &Value) -> (String, String) {
    if let Some(v) = prop["valueCode"].as_str() {
        return ("code".into(), v.into());
    }
    if let Some(v) = prop["valueBoolean"].as_bool() {
        return ("boolean".into(), v.to_string());
    }
    if let Some(v) = prop["valueInteger"].as_i64() {
        return ("integer".into(), v.to_string());
    }
    if let Some(v) = prop["valueDecimal"].as_f64() {
        return ("decimal".into(), v.to_string());
    }
    if let Some(v) = prop["valueString"].as_str() {
        return ("string".into(), v.into());
    }
    if let Some(v) = prop["valueDateTime"].as_str() {
        return ("dateTime".into(), v.into());
    }
    // Fallback: bare "value" string field (non-standard but seen in some encodings)
    if let Some(v) = prop["value"].as_str() {
        return ("string".into(), v.into());
    }
    ("string".into(), String::new())
}

/// Insert one concept designation row.
#[cfg(feature = "sqlite")]
fn import_designation(conn: &Connection, concept_id: i64, desig: &Value) -> Result<(), HtsError> {
    let language = desig["language"].as_str().map(str::to_owned);
    let use_system = desig["use"]["system"].as_str().map(str::to_owned);
    let use_code = desig["use"]["code"].as_str().map(str::to_owned);
    let value = match desig["value"].as_str() {
        Some(v) => v.to_owned(),
        None => return Ok(()), // skip malformed designation
    };

    conn.execute(
        "INSERT INTO concept_designations (concept_id, language, use_system, use_code, value)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![concept_id, language, use_system, use_code, value],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(())
}

/// Insert a single `parent_code → child_code` row into `concept_hierarchy`,
/// ignoring duplicates (`INSERT OR IGNORE`).
#[cfg(feature = "sqlite")]
fn insert_hierarchy(
    conn: &Connection,
    system_id: &str,
    parent_code: &str,
    child_code: &str,
) -> Result<(), HtsError> {
    conn.execute(
        "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![system_id, parent_code, child_code],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    Ok(())
}

// ── ValueSet ───────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
pub(crate) fn import_value_set(
    conn: &Connection,
    vs: &Value,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let id = vs["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let url = vs["url"]
        .as_str()
        .ok_or_else(|| HtsError::InvalidRequest("ValueSet.url is required".into()))?;

    let version = vs["version"].as_str();
    let name = vs["name"].as_str();
    let title = vs["title"].as_str();
    let status = vs["status"].as_str().unwrap_or("active");

    // Store the raw compose element as JSON; expansion is deferred to the first
    // $expand call using a lazy-expansion strategy.
    let compose_json = vs["compose"]
        .as_object()
        .map(|_| serde_json::to_string(&vs["compose"]).unwrap_or_default());

    let resource_json = serde_json::to_string(vs).ok();
    let now = utc_now();

    conn.execute(
        "INSERT OR REPLACE INTO value_sets
         (id, url, version, name, title, status, compose_json, resource_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        rusqlite::params![id, url, version, name, title, status, compose_json, resource_json, now],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    stats.value_sets += 1;
    Ok(())
}

// ── ConceptMap ─────────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
pub(crate) fn import_concept_map(
    conn: &Connection,
    cm: &Value,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let id = cm["id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let url = cm["url"]
        .as_str()
        .ok_or_else(|| HtsError::InvalidRequest("ConceptMap.url is required".into()))?;

    let version = cm["version"].as_str();
    let name = cm["name"].as_str();
    let title = cm["title"].as_str();
    // FHIR R4 uses sourceUri/targetUri; R5 uses sourceScope/targetScope.
    let source_uri = cm["sourceUri"]
        .as_str()
        .or_else(|| cm["sourceScope"].as_str());
    let target_uri = cm["targetUri"]
        .as_str()
        .or_else(|| cm["targetScope"].as_str());
    let status = cm["status"].as_str().unwrap_or("active");
    let resource_json = serde_json::to_string(cm).ok();
    let now = utc_now();

    conn.execute(
        "INSERT OR REPLACE INTO concept_maps
         (id, url, version, name, title, source_uri, target_uri, status, resource_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            id,
            url,
            version,
            name,
            title,
            source_uri,
            target_uri,
            status,
            resource_json,
            now
        ],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Import group elements.
    let empty = vec![];
    let groups = cm["group"].as_array().unwrap_or(&empty);

    for group in groups {
        let source_system = group["source"].as_str().unwrap_or("");
        let target_system = group["target"].as_str().unwrap_or("");
        let elements = group["element"].as_array().unwrap_or(&empty);

        for element in elements {
            let source_code = element["code"].as_str().unwrap_or("");
            let element_targets = element["target"].as_array().unwrap_or(&empty);

            for target in element_targets {
                let target_code = target["code"].as_str().unwrap_or("");
                let equivalence = target["equivalence"].as_str().unwrap_or("equivalent");

                if source_code.is_empty() || target_code.is_empty() {
                    continue;
                }

                conn.execute(
                    "INSERT OR IGNORE INTO concept_map_elements
                     (map_id, source_system, source_code, target_system, target_code, equivalence)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        id,
                        source_system,
                        source_code,
                        target_system,
                        target_code,
                        equivalence
                    ],
                )
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
            }
        }
    }

    stats.concept_maps += 1;
    Ok(())
}

// ── Normalized-table delete helpers (used by CRUD handlers) ───────────────────

/// Look up a CodeSystem's canonical URL by its FHIR resource `id`.
///
/// Returns `Ok(None)` when no code system with that `id` exists.
#[cfg(feature = "sqlite")]
pub(crate) fn get_code_system_url(conn: &Connection, id: &str) -> Result<Option<String>, HtsError> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT url FROM code_systems WHERE id = ?1",
        rusqlite::params![id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| HtsError::StorageError(e.to_string()))
}

/// Delete all cached value set expansion rows that were derived from the given
/// code system URL.
///
/// Called before a CodeSystem is updated or deleted to prevent stale cached
/// expansions from being returned by subsequent `$expand` calls.
/// The expansion will be recomputed on the next `$expand` invocation.
#[cfg(feature = "sqlite")]
pub(crate) fn invalidate_expansion_cache_for_system(
    conn: &Connection,
    system_url: &str,
) -> Result<(), HtsError> {
    conn.execute(
        "DELETE FROM value_set_expansions WHERE system_url = ?1",
        rusqlite::params![system_url],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(())
}

/// Delete a CodeSystem and all its normalized data (concepts, hierarchy,
/// properties, designations) by its FHIR resource `id`.
///
/// ON DELETE CASCADE in the schema propagates the deletion to child rows.
/// If no row with the given `id` exists the operation is a silent no-op.
#[cfg(feature = "sqlite")]
pub(crate) fn delete_code_system(conn: &Connection, id: &str) -> Result<(), HtsError> {
    conn.execute(
        "DELETE FROM code_systems WHERE id = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(())
}

/// Delete a ValueSet and its materialized expansion cache by FHIR resource `id`.
#[cfg(feature = "sqlite")]
pub(crate) fn delete_value_set(conn: &Connection, id: &str) -> Result<(), HtsError> {
    conn.execute(
        "DELETE FROM value_sets WHERE id = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(())
}

/// Delete a ConceptMap and all its element rows by FHIR resource `id`.
#[cfg(feature = "sqlite")]
pub(crate) fn delete_concept_map(conn: &Connection, id: &str) -> Result<(), HtsError> {
    conn.execute(
        "DELETE FROM concept_maps WHERE id = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(())
}

// ── Timestamp helper ───────────────────────────────────────────────────────────

fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::backend::sqlite::SqliteTerminologyBackend;
    use crate::import::BundleImportBackend;
    use crate::traits::CodeSystemOperations;
    use crate::types::LookupRequest;
    use helios_persistence::tenant::TenantContext;

    fn backend() -> SqliteTerminologyBackend {
        SqliteTerminologyBackend::in_memory().expect("in-memory backend should initialise")
    }

    fn ctx() -> TenantContext {
        TenantContext::system()
    }

    /// A minimal synthetic bundle containing one CodeSystem, one ValueSet, and
    /// one ConceptMap.
    fn minimal_bundle_json() -> &'static str {
        r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-test",
                "url": "http://example.org/cs",
                "version": "1.0",
                "name": "TestCS",
                "status": "active",
                "content": "complete",
                "concept": [
                  {
                    "code": "A",
                    "display": "Concept A",
                    "definition": "First concept",
                    "concept": [
                      {
                        "code": "B",
                        "display": "Concept B",
                        "concept": [
                          { "code": "C", "display": "Concept C" }
                        ]
                      }
                    ]
                  }
                ]
              }
            },
            {
              "resource": {
                "resourceType": "ValueSet",
                "id": "vs-test",
                "url": "http://example.org/vs",
                "version": "1.0",
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
            },
            {
              "resource": {
                "resourceType": "ConceptMap",
                "id": "cm-test",
                "url": "http://example.org/cm",
                "version": "1.0",
                "status": "active",
                "sourceUri": "http://example.org/cs",
                "targetUri": "http://example.org/other",
                "group": [
                  {
                    "source": "http://example.org/cs",
                    "target": "http://example.org/other",
                    "element": [
                      {
                        "code": "A",
                        "target": [{ "code": "X", "equivalence": "equivalent" }]
                      }
                    ]
                  }
                ]
              }
            }
          ]
        }"#
    }

    // ── Unit: import stats ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn import_stats_counts_all_resources() {
        let b = backend();
        let stats = b
            .import_bundle(&ctx(), minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.value_sets, 1);
        assert_eq!(stats.concept_maps, 1);
        assert_eq!(stats.concepts, 3); // A, B, C
        assert!(!stats.has_errors(), "errors: {:?}", stats.errors);
    }

    // ── Unit: non-bundle JSON returns error ────────────────────────────────────

    #[tokio::test]
    async fn non_bundle_json_returns_invalid_request() {
        let b = backend();
        let patient = r#"{"resourceType":"Patient","id":"p1"}"#;
        let err = b
            .import_bundle(&ctx(), patient.as_bytes())
            .await
            .unwrap_err();
        assert!(matches!(err, HtsError::InvalidRequest(_)));
    }

    // ── Unit: invalid JSON returns error ───────────────────────────────────────

    #[tokio::test]
    async fn invalid_json_returns_invalid_request() {
        let b = backend();
        let err = b
            .import_bundle(&ctx(), b"not json at all")
            .await
            .unwrap_err();
        assert!(matches!(err, HtsError::InvalidRequest(_)));
    }

    // ── Unit: CodeSystem missing url is recorded as non-fatal error ────────────

    #[tokio::test]
    async fn code_system_missing_url_is_non_fatal_error() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            { "resource": { "resourceType": "CodeSystem", "id": "cs1", "status": "active" } }
          ]
        }"#;
        let stats = b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();
        assert_eq!(stats.code_systems, 0);
        assert!(stats.has_errors());
    }

    // ── Unit: hierarchy from nested concepts ───────────────────────────────────

    #[tokio::test]
    async fn hierarchy_from_nesting_is_materialised() {
        let b = backend();
        b.import_bundle(&ctx(), minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        let conn = b.pool().get().unwrap();

        // A→B should be stored (direct edge from nesting).
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_hierarchy
                 WHERE system_id='cs-test' AND parent_code='A' AND child_code='B'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "A→B edge should be materialised");

        // B→C should be stored.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_hierarchy
                 WHERE system_id='cs-test' AND parent_code='B' AND child_code='C'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "B→C edge should be materialised");
    }

    // ── Unit: hierarchy from parent property ───────────────────────────────────

    #[tokio::test]
    async fn hierarchy_from_parent_property_is_materialised() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-prop",
                "url": "http://example.org/cs-prop",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "ROOT", "display": "Root" },
                  {
                    "code": "CHILD",
                    "display": "Child",
                    "property": [
                      { "code": "parent", "valueCode": "ROOT" }
                    ]
                  }
                ]
              }
            }
          ]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let conn = b.pool().get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_hierarchy
                 WHERE system_id='cs-prop' AND parent_code='ROOT' AND child_code='CHILD'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "ROOT→CHILD edge should be materialised");
    }

    // ── Unit: value set compose JSON is stored ─────────────────────────────────

    #[tokio::test]
    async fn value_set_compose_json_is_stored() {
        let b = backend();
        b.import_bundle(&ctx(), minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        let conn = b.pool().get().unwrap();
        let compose_json: Option<String> = conn
            .query_row(
                "SELECT compose_json FROM value_sets WHERE url='http://example.org/vs'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        assert!(
            compose_json.is_some(),
            "compose_json should be stored for the value set"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(compose_json.as_deref().unwrap()).unwrap();
        assert!(
            parsed["include"].is_array(),
            "stored compose should be valid JSON with 'include'"
        );
    }

    // ── Unit: concept map elements are stored ──────────────────────────────────

    #[tokio::test]
    async fn concept_map_elements_are_stored() {
        let b = backend();
        b.import_bundle(&ctx(), minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        let conn = b.pool().get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_map_elements WHERE map_id='cm-test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "exactly one mapping element should be stored");

        let (src, tgt, eq): (String, String, String) = conn
            .query_row(
                "SELECT source_code, target_code, equivalence
                 FROM concept_map_elements WHERE map_id='cm-test'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(src, "A");
        assert_eq!(tgt, "X");
        assert_eq!(eq, "equivalent");
    }

    // ── Integration: import Bundle → $lookup ───────────────────────────────────

    #[tokio::test]
    async fn lookup_after_import_returns_correct_display() {
        let b = backend();
        b.import_bundle(&ctx(), minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        let resp = b
            .lookup(
                &ctx(),
                LookupRequest {
                    system: "http://example.org/cs".into(),
                    code: "A".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.display, Some("Concept A".into()));
        assert_eq!(resp.name, "TestCS");
    }

    // ── Integration: import Bundle → subsumes via pre-materialised hierarchy ───

    #[tokio::test]
    async fn subsumes_works_after_import() {
        use crate::traits::CodeSystemOperations;
        use crate::types::{SubsumesRequest, SubsumptionOutcome};

        let b = backend();
        b.import_bundle(&ctx(), minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        let resp = b
            .subsumes(
                &ctx(),
                SubsumesRequest {
                    system: "http://example.org/cs".into(),
                    version: None,
                    code_a: "A".into(),
                    code_b: "C".into(),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            resp.outcome,
            SubsumptionOutcome::Subsumes,
            "A should subsume C via A→B→C hierarchy"
        );
    }

    // ── Unit: designations are stored ─────────────────────────────────────────

    #[tokio::test]
    async fn designations_are_stored() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-desig",
                "url": "http://example.org/cs-desig",
                "status": "active",
                "content": "complete",
                "concept": [
                  {
                    "code": "ALPHA",
                    "display": "Alpha",
                    "designation": [
                      {
                        "language": "fr",
                        "use": { "system": "http://snomed.info/sct", "code": "900000000000003001" },
                        "value": "Alpha (fr)"
                      }
                    ]
                  }
                ]
              }
            }
          ]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let conn = b.pool().get().unwrap();
        let (lang, val): (Option<String>, String) = conn
            .query_row(
                "SELECT cd.language, cd.value
                 FROM concept_designations cd
                 JOIN concepts c ON c.id = cd.concept_id
                 WHERE c.code = 'ALPHA'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(lang.as_deref(), Some("fr"));
        assert_eq!(val, "Alpha (fr)");
    }

    // ── Unit: empty bundle returns zero stats ──────────────────────────────────

    #[tokio::test]
    async fn empty_bundle_returns_zero_stats() {
        let b = backend();
        let bundle = r#"{ "resourceType": "Bundle", "type": "collection" }"#;
        let stats = b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        assert_eq!(stats.code_systems, 0);
        assert_eq!(stats.value_sets, 0);
        assert_eq!(stats.concept_maps, 0);
        assert_eq!(stats.concepts, 0);
        assert!(!stats.has_errors());
    }

    // ── Unit: reimporting the same bundle is idempotent ────────────────────────

    #[tokio::test]
    async fn reimport_is_idempotent() {
        let b = backend();
        b.import_bundle(&ctx(), minimal_bundle_json().as_bytes())
            .await
            .unwrap();
        let stats = b
            .import_bundle(&ctx(), minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        assert_eq!(stats.code_systems, 1);
        assert!(!stats.has_errors(), "reimport should not produce errors");

        // Exactly 3 concepts should exist — no duplicates.
        let conn = b.pool().get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE system_id='cs-test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3, "duplicate concepts should not be created");
    }

    // ── Unit: concept properties are stored ───────────────────────────────────

    #[tokio::test]
    async fn concept_properties_are_stored() {
        let b = backend();
        let bundle = r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "cs-props",
                "url": "http://example.org/cs-props",
                "status": "active",
                "content": "complete",
                "concept": [
                  {
                    "code": "CODE1",
                    "display": "Code One",
                    "property": [
                      { "code": "inactive", "valueBoolean": true },
                      { "code": "comment", "valueString": "test comment" }
                    ]
                  }
                ]
              }
            }
          ]
        }"#;

        b.import_bundle(&ctx(), bundle.as_bytes()).await.unwrap();

        let conn = b.pool().get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_properties cp
                 JOIN concepts c ON c.id = cp.concept_id
                 WHERE c.code = 'CODE1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "both properties should be stored");
    }
}
