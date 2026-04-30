//! SQLite FHIR Bundle write layer.
//!
//! Consumes the backend-agnostic [`ParsedBundle`] produced by
//! [`bundle_parser::parse_bundle`] and writes every resource into the
//! SQLite normalized terminology tables.
//!
//! Import order is always: CodeSystems → ValueSets → ConceptMaps, matching
//! the order guaranteed by [`bundle_parser::parse_bundle`].

#[cfg(feature = "sqlite")]
use crate::backends::sqlite::schema;
use crate::error::HtsError;
use crate::import::ImportStats;
use crate::import::bundle_parser::{
    self, ParsedBundle, ParsedCodeSystem, ParsedConceptMap, ParsedValueSet,
};
#[cfg(feature = "sqlite")]
use r2d2::Pool;
#[cfg(feature = "sqlite")]
use r2d2_sqlite::SqliteConnectionManager;
#[cfg(feature = "sqlite")]
use rusqlite::{Connection, OptionalExtension};

// ── Public entry point ────────────────────────────────────────────────────────

/// Parse a FHIR Bundle from raw bytes and insert its resources into SQLite.
///
/// Called by `SqliteTerminologyBackend::import_bundle` and the `POST /import`
/// HTTP handler.
#[cfg(feature = "sqlite")]
pub(crate) fn import_bundle_sync(
    pool: &Pool<SqliteConnectionManager>,
    data: &[u8],
) -> Result<ImportStats, HtsError> {
    let parsed = bundle_parser::parse_bundle(data)?;
    let mut conn = pool
        .get()
        .map_err(|e| HtsError::StorageError(format!("Pool error: {e}")))?;
    let mut stats = ImportStats::default();

    // Before the transaction: record which code systems currently have zero
    // concepts in the DB. After the transaction commits, only these systems
    // get an immediate closure rebuild — they are either brand-new systems or
    // empty stubs, so the build is fast (at most a few thousand pairs).
    //
    // Systems that already have concepts are being updated in a batch (e.g.
    // SNOMED RF2 chunks). Building the closure after every batch is O(n²) for
    // SNOMED CT (~640K concepts, ~1 280 batches = hours). Skipping per-batch
    // rebuilds is safe: write_code_system deletes the stale closure, and
    // migrate_concept_closure at server startup rebuilds it exactly once.
    let systems_needing_closure: Vec<String> = parsed
        .code_systems
        .iter()
        .filter_map(|cs| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM concepts c
                     JOIN code_systems s ON c.system_id = s.id
                     WHERE s.url = ?1",
                    rusqlite::params![cs.url],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if count == 0 {
                Some(cs.url.clone())
            } else {
                None
            }
        })
        .collect();

    // Wrap the whole bundle in a single transaction so that the thousands of
    // per-concept / per-property / per-designation inserts that a bulk
    // terminology load produces commit once, not once per row. Combined with
    // `prepare_cached` inside the `write_*` helpers, this is the dominant
    // speed-up for `hts import`.
    let tx = conn
        .transaction()
        .map_err(|e| HtsError::StorageError(format!("Begin transaction: {e}")))?;
    write_parsed_bundle(&tx, &parsed, &mut stats)?;
    tx.commit()
        .map_err(|e| HtsError::StorageError(format!("Commit transaction: {e}")))?;

    // Rebuild concept closure for newly imported (previously empty) code systems.
    // Skipped for batch imports of existing systems (see comment above).
    for url in &systems_needing_closure {
        let system_id: Option<String> = conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = ?1",
                rusqlite::params![url],
                |r| r.get(0),
            )
            .ok();
        if let Some(sid) = system_id {
            let has_hierarchy: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM concept_hierarchy WHERE system_id = ?1 LIMIT 1)",
                    rusqlite::params![sid],
                    |r| r.get(0),
                )
                .unwrap_or(false);
            if has_hierarchy {
                if let Err(e) = schema::build_concept_closure(&conn, &sid) {
                    tracing::warn!(system_id = %sid, error = %e, "Failed to build concept closure after import");
                }
            }
        }
    }

    Ok(stats)
}

/// Write a [`ParsedBundle`] into the SQLite normalized tables.
///
/// All three resource types are processed in dependency order:
/// CodeSystems first, then ValueSets, then ConceptMaps.
#[cfg(feature = "sqlite")]
pub(crate) fn write_parsed_bundle(
    conn: &Connection,
    parsed: &ParsedBundle,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    stats.errors.extend(parsed.parse_errors.iter().cloned());
    for cs in &parsed.code_systems {
        let url = cs.url.as_str();
        if let Err(e) = write_code_system(conn, cs, stats) {
            stats
                .errors
                .push(format!("CodeSystem '{url}' import failed: {e}"));
        }
    }
    for vs in &parsed.value_sets {
        let url = vs.url.as_str();
        if let Err(e) = write_value_set(conn, vs, stats) {
            stats
                .errors
                .push(format!("ValueSet '{url}' import failed: {e}"));
        }
    }
    for cm in &parsed.concept_maps {
        let url = cm.url.as_str();
        if let Err(e) = write_concept_map(conn, cm, stats) {
            stats
                .errors
                .push(format!("ConceptMap '{url}' import failed: {e}"));
        }
    }
    Ok(())
}

// ── CodeSystem write ──────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
pub(crate) fn import_code_system(
    conn: &Connection,
    cs_json: &serde_json::Value,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    if let Some(parsed) = bundle_parser::parse_code_system_value(cs_json) {
        write_code_system(conn, &parsed, stats)
    } else {
        Err(HtsError::InvalidRequest(
            "CodeSystem.url is required".into(),
        ))
    }
}

#[cfg(feature = "sqlite")]
fn write_code_system(
    conn: &Connection,
    cs: &ParsedCodeSystem,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let resource_json = serde_json::to_string(&cs.resource_json).ok();
    let now = utc_now();

    // Resolve the id to use:
    //   1. If a row for this URL already exists, reuse its id (in-place update).
    //   2. Otherwise use cs.id, unless cs.id is already taken by a different URL
    //      (can happen when two distinct CodeSystems share a short FHIR id like
    //      "observation-status") — in that case mint a fresh UUID.
    let existing_id: Option<String> = conn
        .prepare_cached("SELECT id FROM code_systems WHERE url = ?1")
        .and_then(|mut s| {
            s.query_row(rusqlite::params![cs.url], |row| row.get(0))
                .optional()
        })
        .map_err(|e| HtsError::StorageError(e.to_string()))?;

    let system_id: String = if let Some(id) = existing_id {
        id
    } else {
        let id_taken: bool = conn
            .prepare_cached("SELECT COUNT(*) FROM code_systems WHERE id = ?1")
            .and_then(|mut s| s.query_row(rusqlite::params![cs.id], |row| row.get::<_, i64>(0)))
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            > 0;
        if id_taken {
            uuid::Uuid::new_v4().to_string()
        } else {
            cs.id.clone()
        }
    };

    // Use ON CONFLICT(url) DO UPDATE instead of INSERT OR REPLACE to avoid
    // triggering the ON DELETE CASCADE on concepts — a stub CodeSystem from
    // an hl7-npm package (content=not-present) must never wipe existing
    // RF2/LOINC concepts that were imported separately.
    conn.prepare_cached(
        "INSERT INTO code_systems
         (id, url, version, name, title, status, content, resource_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(url) DO UPDATE SET
             version       = excluded.version,
             name          = excluded.name,
             title         = excluded.title,
             status        = excluded.status,
             content       = excluded.content,
             resource_json = excluded.resource_json,
             updated_at    = excluded.updated_at",
    )
    .and_then(|mut s| {
        s.execute(rusqlite::params![
            system_id,
            cs.url,
            cs.version,
            cs.name,
            cs.title,
            cs.status,
            cs.content,
            resource_json,
            now
        ])
    })
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Upsert the concept and read back its id in one round-trip via
    // `RETURNING id` — replacing the previous INSERT OR REPLACE +
    // last_insert_rowid() pattern, which forced a synchronous round-trip
    // after every row. ON CONFLICT here avoids the row delete/reinsert that
    // INSERT OR REPLACE would cause (and the cascade on concept_id children).
    const UPSERT_CONCEPT_SQL: &str = "INSERT INTO concepts (system_id, code, display, definition)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(system_id, code) DO UPDATE SET
             display    = excluded.display,
             definition = excluded.definition
         RETURNING id";
    const INSERT_PROPERTY_SQL: &str =
        "INSERT INTO concept_properties (concept_id, property, value_type, value)
         VALUES (?1, ?2, ?3, ?4)";
    const INSERT_DESIGNATION_SQL: &str = "INSERT INTO concept_designations
         (concept_id, language, use_system, use_code, value)
         VALUES (?1, ?2, ?3, ?4, ?5)";

    for concept in &cs.concepts {
        let concept_id: i64 = conn
            .prepare_cached(UPSERT_CONCEPT_SQL)
            .and_then(|mut s| {
                s.query_row(
                    rusqlite::params![system_id, concept.code, concept.display, concept.definition],
                    |row| row.get(0),
                )
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?;

        // Hierarchy from nesting or "parent" property.
        if let Some(ref parent) = concept.parent_code {
            insert_hierarchy(conn, &system_id, parent, &concept.code)?;
        }

        // Properties.
        // Delete existing rows first so reimports stay idempotent.  We only do
        // this when the incoming concept carries at least one non-empty property
        // so that stub "content=not-present" re-imports don't wipe RF2/LOINC
        // properties that were loaded separately.
        let has_props = concept.properties.iter().any(|p| !p.value.is_empty());
        if has_props {
            conn.execute(
                "DELETE FROM concept_properties WHERE concept_id = ?1",
                rusqlite::params![concept_id],
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        }
        for prop in &concept.properties {
            if prop.value.is_empty() {
                continue;
            }
            conn.prepare_cached(INSERT_PROPERTY_SQL)
                .and_then(|mut s| {
                    s.execute(rusqlite::params![
                        concept_id,
                        prop.code,
                        prop.value_type,
                        prop.value
                    ])
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?;

            // Extra hierarchy edge from a "parent" property.
            if prop.is_parent_edge {
                if let Some(ref pv) = prop.parent_code_value {
                    insert_hierarchy(conn, &system_id, pv, &concept.code)?;
                }
            }
        }

        // Designations — same idempotency guard.
        let has_desigs = !concept.designations.is_empty();
        if has_desigs {
            conn.execute(
                "DELETE FROM concept_designations WHERE concept_id = ?1",
                rusqlite::params![concept_id],
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
        }
        for desig in &concept.designations {
            conn.prepare_cached(INSERT_DESIGNATION_SQL)
                .and_then(|mut s| {
                    s.execute(rusqlite::params![
                        concept_id,
                        desig.language,
                        desig.use_system,
                        desig.use_code,
                        desig.value
                    ])
                })
                .map_err(|e| HtsError::StorageError(e.to_string()))?;
        }

        stats.concepts += 1;
    }

    // Invalidate stale closure rows so that migrate_concept_closure at server
    // startup knows to (re)build the full closure once all batches are loaded.
    // Without this, a previous partial closure (from a re-import or an earlier
    // batch in the same session) would be mistakenly treated as complete.
    let _ = conn.execute(
        "DELETE FROM concept_closure WHERE system_id = ?1",
        rusqlite::params![system_id],
    );

    // Invalidate any cached implicit-ValueSet expansions for this code system.
    // The implicit_expansion_cache is otherwise persistent across restarts; stale
    // entries from a previous version of this system must be evicted on re-import.
    let _ = conn.execute(
        "DELETE FROM implicit_expansion_cache WHERE system_url = ?1",
        rusqlite::params![cs.url],
    );
    let _ = conn.execute(
        "DELETE FROM implicit_expansion_fts WHERE system_url = ?1",
        rusqlite::params![cs.url],
    );

    stats.code_systems += 1;
    Ok(())
}

#[cfg(feature = "sqlite")]
fn insert_hierarchy(
    conn: &Connection,
    system_id: &str,
    parent_code: &str,
    child_code: &str,
) -> Result<(), HtsError> {
    conn.prepare_cached(
        "INSERT OR IGNORE INTO concept_hierarchy (system_id, parent_code, child_code)
         VALUES (?1, ?2, ?3)",
    )
    .and_then(|mut s| s.execute(rusqlite::params![system_id, parent_code, child_code]))
    .map_err(|e| HtsError::StorageError(e.to_string()))?;
    Ok(())
}

// ── ValueSet write ────────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
pub(crate) fn import_value_set(
    conn: &Connection,
    vs_json: &serde_json::Value,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    if let Some(parsed) = bundle_parser::parse_value_set_value(vs_json) {
        write_value_set(conn, &parsed, stats)
    } else {
        Err(HtsError::InvalidRequest("ValueSet.url is required".into()))
    }
}

#[cfg(feature = "sqlite")]
fn write_value_set(
    conn: &Connection,
    vs: &ParsedValueSet,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let resource_json = serde_json::to_string(&vs.resource_json).ok();
    let now = utc_now();

    conn.prepare_cached(
        "INSERT OR REPLACE INTO value_sets
         (id, url, version, name, title, status, compose_json, resource_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
    )
    .and_then(|mut s| {
        s.execute(rusqlite::params![
            vs.id,
            vs.url,
            vs.version,
            vs.name,
            vs.title,
            vs.status,
            vs.compose_json,
            resource_json,
            now
        ])
    })
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    stats.value_sets += 1;
    Ok(())
}

// ── ConceptMap write ──────────────────────────────────────────────────────────

#[cfg(feature = "sqlite")]
pub(crate) fn import_concept_map(
    conn: &Connection,
    cm_json: &serde_json::Value,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    if let Some(parsed) = bundle_parser::parse_concept_map_value(cm_json) {
        write_concept_map(conn, &parsed, stats)
    } else {
        Err(HtsError::InvalidRequest(
            "ConceptMap.url is required".into(),
        ))
    }
}

#[cfg(feature = "sqlite")]
fn write_concept_map(
    conn: &Connection,
    cm: &ParsedConceptMap,
    stats: &mut ImportStats,
) -> Result<(), HtsError> {
    let resource_json = serde_json::to_string(&cm.resource_json).ok();
    let now = utc_now();

    conn.prepare_cached(
        "INSERT OR REPLACE INTO concept_maps
         (id, url, version, name, title, source_uri, target_uri, status, resource_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .and_then(|mut s| {
        s.execute(rusqlite::params![
            cm.id,
            cm.url,
            cm.version,
            cm.name,
            cm.title,
            cm.source_uri,
            cm.target_uri,
            cm.status,
            resource_json,
            now
        ])
    })
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    const INSERT_ELEMENT_SQL: &str = "INSERT OR IGNORE INTO concept_map_elements
         (map_id, source_system, source_code, target_system, target_code, equivalence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)";
    for el in &cm.elements {
        conn.prepare_cached(INSERT_ELEMENT_SQL)
            .and_then(|mut s| {
                s.execute(rusqlite::params![
                    cm.id,
                    el.source_system,
                    el.source_code,
                    el.target_system,
                    el.target_code,
                    el.equivalence
                ])
            })
            .map_err(|e| HtsError::StorageError(e.to_string()))?;
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

/// Delete a CodeSystem and all its normalized data by its FHIR resource `id`.
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
    use crate::backends::sqlite::SqliteTerminologyBackend;
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
                "status": "active",
                "compose": {
                  "include": [{ "system": "http://example.org/cs" }]
                }
              }
            },
            {
              "resource": {
                "resourceType": "ConceptMap",
                "id": "cm-test",
                "url": "http://example.org/cm",
                "status": "active",
                "group": [{
                  "source": "http://example.org/cs",
                  "target": "http://example.org/other",
                  "element": [{
                    "code": "A",
                    "target": [{ "code": "Z", "equivalence": "equivalent" }]
                  }]
                }]
              }
            }
          ]
        }"#
    }

    #[tokio::test]
    async fn import_bundle_inserts_all_resource_types() {
        let b = backend();
        let ctx = ctx();
        let stats = b
            .import_bundle(&ctx, minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        assert_eq!(stats.code_systems, 1);
        assert_eq!(stats.value_sets, 1);
        assert_eq!(stats.concept_maps, 1);
        assert_eq!(stats.concepts, 3); // A + B + C
        assert!(stats.errors.is_empty());
    }

    #[tokio::test]
    async fn import_then_lookup_end_to_end() {
        let b = backend();
        let ctx = ctx();
        b.import_bundle(&ctx, minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        let resp = b
            .lookup(
                &ctx,
                LookupRequest {
                    system: "http://example.org/cs".into(),
                    code: "A".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(resp.display.as_deref(), Some("Concept A"));
    }

    #[tokio::test]
    async fn import_invalid_json_returns_error() {
        let b = backend();
        let ctx = ctx();
        let result = b.import_bundle(&ctx, b"not json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn import_non_bundle_returns_error() {
        let b = backend();
        let ctx = ctx();
        let data = br#"{"resourceType":"Patient","id":"p1"}"#;
        let result = b.import_bundle(&ctx, data).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn hierarchy_materialized_from_nesting() {
        let b = backend();
        let ctx = ctx();
        b.import_bundle(&ctx, minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        let conn = b.pool().get().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_hierarchy WHERE system_id='cs-test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // A→B and B→C
        assert_eq!(count, 2, "Two hierarchy edges should be materialized");
    }
}
