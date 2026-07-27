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
    import_parsed_sync(pool, &parsed)
}

/// Insert an already-parsed bundle into SQLite. Shared by
/// [`import_bundle_sync`] and the direct
/// [`BundleImportBackend::import_parsed`](crate::import::BundleImportBackend::import_parsed)
/// path used by the chunked filesystem importers.
#[cfg(feature = "sqlite")]
pub(crate) fn import_parsed_sync(
    pool: &Pool<SqliteConnectionManager>,
    parsed: &ParsedBundle,
) -> Result<ImportStats, HtsError> {
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
    // Probed with EXISTS rather than COUNT(*): the answer only distinguishes
    // empty from non-empty, and a COUNT over an ever-growing concept set would
    // make each batch of a chunked bulk load (SNOMED RF2, LOINC) scan all
    // previously imported rows.
    let systems_needing_closure: Vec<String> = parsed
        .code_systems
        .iter()
        .filter_map(|cs| {
            let has_concepts: bool = conn
                .query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM concepts c
                         JOIN code_systems s ON c.system_id = s.id
                         WHERE s.url = ?1
                     )",
                    rusqlite::params![cs.url],
                    |r| r.get(0),
                )
                .unwrap_or(false);
            if has_concepts {
                None
            } else {
                Some(cs.url.clone())
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
    write_parsed_bundle(&tx, parsed, &mut stats)?;
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
        if let Err(e) = write_code_system(conn, cs, stats, parsed.fresh_load) {
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
        // CRUD single-resource path: the target may already hold concepts, so
        // always replace (never skip the child delete).
        write_code_system(conn, &parsed, stats, false)
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
    // When `true` the caller guarantees this code system had no concepts before
    // the import session, so each concept's properties/designations cannot have
    // pre-existing rows and the delete-before-reinsert is skipped (fast path for
    // first-time bulk loads). See [`ParsedBundle::fresh_load`].
    skip_child_delete: bool,
) -> Result<(), HtsError> {
    let resource_json = serde_json::to_string(&cs.resource_json).ok();
    let now = utc_now();

    // Synthetic storage id: `<fhir-id>|<version>` (or `<fhir-id>` when version
    // is absent). This guarantees distinct rows per (url, version) even when
    // the upstream resource ships the same FHIR `id` for multiple versions
    // (e.g. tx-ecosystem `version/codesystem-version-1.json` + `-2.json` both
    // declare `"id":"version"`). The pipe character is reserved in canonical
    // URLs so it cannot collide with a legitimate FHIR id.
    //
    // When two distinct CodeSystems share both fhir-id AND version (e.g. two
    // unrelated CSes ship `id`="status" with no version), reuse the existing
    // row for the matching (url, version) or mint a fresh UUID rather than
    // letting the second import collide on the primary key and silently get
    // dropped by INSERT OR IGNORE.
    let preferred_id = storage_id_for(&cs.id, cs.version.as_deref());
    let existing_for_url_version: Option<String> = conn
        .query_row(
            "SELECT id FROM code_systems \
             WHERE url = ?1 AND COALESCE(version, '') = COALESCE(?2, '')",
            rusqlite::params![cs.url, cs.version],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    let storage_id = if let Some(id) = existing_for_url_version {
        id
    } else {
        let preferred_taken: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM code_systems WHERE id = ?1",
                rusqlite::params![preferred_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            > 0;
        if preferred_taken {
            uuid::Uuid::new_v4().to_string()
        } else {
            preferred_id
        }
    };

    // Upsert keyed on (url, version): a re-import of the same version updates
    // the existing row rather than creating a new one or wiping sibling
    // versions. The composite UNIQUE index on (url, COALESCE(version,''))
    // guarantees each (url, version) maps to at most one storage row.
    conn.execute(
        "INSERT OR IGNORE INTO code_systems
         (id, url, version, name, title, status, content, resource_json, created_at, updated_at,
          authority_rank)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
        rusqlite::params![
            storage_id,
            cs.url,
            cs.version,
            cs.name,
            cs.title,
            cs.status,
            cs.content,
            resource_json,
            now,
            cs.authority_rank,
        ],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // INSERT OR IGNORE skips the update path on conflict; force-update the
    // metadata for this (url, version) row so re-imports refresh title/status
    // /resource_json without disturbing sibling versions.
    //
    // `authority_rank` keeps the strongest claim ever asserted for this row: if a
    // source that owns this canonical URL supplies it, the row stays authoritative
    // even when a package that merely re-publishes it is imported afterwards. That
    // makes the RANK independent of import order — which matters, because bootstrap
    // walks the directory alphabetically and `hl7.fhir.r4.core-*.tgz` happens to
    // sort before `hl7.terminology-*.tgz`. (Only the rank: the other columns are
    // overwritten unconditionally, so whichever source writes a given (url,
    // version) row last supplies its content. No two vendored packages ship the
    // same (url, version) — verified across all 8 — so that is moot today.)
    //
    // The COALESCE sentinel (9 — above any real rank) is what makes an existing
    // database converge: a row that predates the column is NULL, meaning "never
    // claimed", so the first source to claim it wins outright. A plain
    // MIN(authority_rank, ?) would instead read the legacy row as rank 0 and pin
    // the stale copy at authoritative forever, silently defeating the migration.
    let cs_update = format!(
        "UPDATE code_systems SET
           name           = ?1,
           title          = ?2,
           status         = ?3,
           content        = ?4,
           resource_json  = ?5,
           updated_at     = ?6,
           authority_rank = MIN(COALESCE(authority_rank, {unclaimed}), ?9)
         WHERE url = ?7 AND COALESCE(version, '') = COALESCE(?8, '')",
        unclaimed = bundle_parser::AUTHORITY_UNCLAIMED
    );
    conn.execute(
        &cs_update,
        rusqlite::params![
            cs.name,
            cs.title,
            cs.status,
            cs.content,
            resource_json,
            now,
            cs.url,
            cs.version,
            cs.authority_rank,
        ],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Resolve the authoritative storage id for this (url, version) pair.
    // A prior import that used a different synthesised FHIR id still wins,
    // so we always look it up via the composite index rather than trusting
    // `storage_id` directly.
    let system_id: String = conn
        .query_row(
            "SELECT id FROM code_systems \
             WHERE url = ?1 AND COALESCE(version, '') = COALESCE(?2, '')",
            rusqlite::params![cs.url, cs.version],
            |row| row.get(0),
        )
        .map_err(|e| HtsError::StorageError(format!("Failed to resolve CodeSystem id: {e}")))?;

    // Upsert each concept with `RETURNING id` to avoid a second round-trip per
    // row.  ON CONFLICT preserves child rows (no cascade-delete) so reimports
    // refresh display/definition without losing properties or designations.
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
        // Delete existing rows first so re-imports fully replace the prior set
        // rather than accumulating or leaving filtered-out rows behind. This is
        // unconditional — even when the incoming concept carries zero non-empty
        // properties — because each importer assembles a concept's complete
        // property/designation set into a single struct before writing, and a
        // concept code appears in exactly one import batch. Stub
        // "content=not-present" seed imports carry an *empty concepts array*, so
        // this loop never runs for them and their separately-loaded data is
        // safe. Skipped entirely on a fresh load, where there is nothing to
        // delete (see `skip_child_delete`).
        if !skip_child_delete {
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

        // Designations — replace for the same reason. This is what makes a
        // narrower HTS_IMPORT_LANGUAGES re-import actually drop the
        // previously-imported translations: a concept that now yields zero
        // designations after filtering must still have its stale rows removed.
        // Skipped on a fresh load (nothing to delete).
        if !skip_child_delete {
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

    // Invalidate this system's concept FTS so a later filtered $expand rebuilds
    // it from the freshly-imported concepts/designations instead of serving
    // stale display/synonym rows. The `concepts_fts_built` tracker row MUST be
    // cleared alongside the content: it is the authoritative "FTS current for
    // this system" flag (both the lazy ensure_concepts_fts path and the
    // startup incremental prebuild_concepts_fts short-circuit on it), so leaving
    // it set would freeze stale FTS results until the next full rebuild.
    // Runs in the same transaction as the concept upserts above, so the content
    // and the tracker flip atomically — no window where a concurrent reader
    // sees "built" with stale rows (issue #295). The three content tables carry
    // an explicit system_id column, so the delete is exact: designation rows
    // (negative `-cd.id` rowids in concepts_search_fts) are removed too, and no
    // other system's rows are touched.
    let _ = conn.execute(
        "DELETE FROM concepts_fts WHERE system_id = ?1",
        rusqlite::params![system_id],
    );
    let _ = conn.execute(
        "DELETE FROM concepts_word_fts WHERE system_id = ?1",
        rusqlite::params![system_id],
    );
    let _ = conn.execute(
        "DELETE FROM concepts_search_fts WHERE system_id = ?1",
        rusqlite::params![system_id],
    );
    let _ = conn.execute(
        "DELETE FROM concepts_fts_built WHERE system_id = ?1",
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

    // The process-wide URL→system_id cache may have memoised a now-stale row
    // (e.g. an empty stub that this import is about to replace, or a
    // re-imported system whose preferred row changed). Drop everything; the
    // cache will repopulate lazily on the next request. The parallel
    // URL→language cache is invalidated alongside.
    crate::backends::sqlite::invalidate_cs_id_cache();
    crate::backends::sqlite::invalidate_cs_language_cache();

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

    // Synthetic storage id: `<fhir-id>|<version>` (or `<fhir-id>` when version
    // is absent). Mirrors the code_systems strategy so multiple ValueSets that
    // share a canonical URL but differ in version don't collide on either the
    // primary key or the composite UNIQUE index. When two distinct VSes share
    // both a fhir-id AND a version (e.g. tx-ecosystem ships several VSes
    // whose `id` is "version-all" but whose canonical URLs differ), reuse the
    // existing row for the matching (url, version) or mint a fresh UUID so
    // the second import doesn't silently get dropped by INSERT OR IGNORE.
    let preferred_id = storage_id_for(&vs.id, vs.version.as_deref());
    let existing_for_url_version: Option<String> = conn
        .query_row(
            "SELECT id FROM value_sets \
             WHERE url = ?1 AND COALESCE(version, '') = COALESCE(?2, '')",
            rusqlite::params![vs.url, vs.version],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?;
    let storage_id = if let Some(id) = existing_for_url_version {
        id
    } else {
        let preferred_taken: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM value_sets WHERE id = ?1",
                rusqlite::params![preferred_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| HtsError::StorageError(e.to_string()))?
            > 0;
        if preferred_taken {
            uuid::Uuid::new_v4().to_string()
        } else {
            preferred_id
        }
    };

    // Drop any materialized expansion derived from the previous content of this
    // ValueSet. `value_set_expansions` cascades on *row delete*, and the upsert
    // below deliberately keeps the row, so without this an expansion computed
    // from the old compose survives a re-import and is served forever. Mirrors
    // what the PostgreSQL `write_value_set` already does. A no-op on first
    // import; one indexed delete on re-import, negligible beside the insert work
    // that follows.
    conn.execute(
        "DELETE FROM value_set_expansions WHERE value_set_id = ?1",
        rusqlite::params![storage_id],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // Upsert keyed on (url, version): a re-import refreshes the existing row
    // for the same version without disturbing sibling versions. The composite
    // UNIQUE index on (url, COALESCE(version,'')) guarantees one storage row
    // per (url, version).
    conn.execute(
        "INSERT OR IGNORE INTO value_sets
         (id, url, version, name, title, status, compose_json, resource_json, created_at, updated_at,
          authority_rank)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)",
        rusqlite::params![
            storage_id,
            vs.url,
            vs.version,
            vs.name,
            vs.title,
            vs.status,
            vs.compose_json,
            resource_json,
            now,
            vs.authority_rank,
        ],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    // INSERT OR IGNORE skipped the metadata refresh on conflict — apply it
    // explicitly so re-imports of the same (url, version) get the latest
    // name/title/status/compose without disturbing siblings.
    // See `write_code_system` for why authority_rank uses MIN over a COALESCE
    // sentinel rather than a plain assignment or a bare MIN.
    let vs_update = format!(
        "UPDATE value_sets SET
           name           = ?1,
           title          = ?2,
           status         = ?3,
           compose_json   = ?4,
           resource_json  = ?5,
           updated_at     = ?6,
           authority_rank = MIN(COALESCE(authority_rank, {unclaimed}), ?9)
         WHERE url = ?7 AND COALESCE(version, '') = COALESCE(?8, '')",
        unclaimed = bundle_parser::AUTHORITY_UNCLAIMED
    );
    conn.execute(
        &vs_update,
        rusqlite::params![
            vs.name,
            vs.title,
            vs.status,
            vs.compose_json,
            resource_json,
            now,
            vs.url,
            vs.version,
            vs.authority_rank,
        ],
    )
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
/// Falls back to matching the original FHIR id stored inside `resource_json`
/// when the synthetic storage id (`<id>|<version>`) doesn't directly match —
/// this is what CRUD callers see in URL paths like `/CodeSystem/version`.
/// When several versions share the same FHIR id we return the latest version
/// (sorted descending as text) so the caller has a defined target.
///
/// Returns `Ok(None)` when no code system with that `id` exists.
#[cfg(feature = "sqlite")]
pub(crate) fn get_code_system_url(conn: &Connection, id: &str) -> Result<Option<String>, HtsError> {
    use rusqlite::OptionalExtension;
    if let Some(url) = conn
        .query_row(
            "SELECT url FROM code_systems WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| HtsError::StorageError(e.to_string()))?
    {
        return Ok(Some(url));
    }
    let sql = format!(
        "SELECT url FROM code_systems \
         WHERE json_extract(resource_json, '$.id') = ?1 \
         ORDER BY {} LIMIT 1",
        crate::backends::cs_precedence_order_by("code_systems")
    );
    conn.query_row(&sql, rusqlite::params![id], |row| row.get::<_, String>(0))
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
///
/// Multi-version: matches both the synthetic storage id (`<id>|<version>`)
/// and the original FHIR id captured in `resource_json.id`, so a CRUD DELETE
/// `/CodeSystem/version` removes every stored version of that resource.
#[cfg(feature = "sqlite")]
pub(crate) fn delete_code_system(conn: &Connection, id: &str) -> Result<(), HtsError> {
    // Collect the storage ids being removed (a multi-version CodeSystem can own
    // several rows sharing one FHIR id) so we can clear their concept FTS rows.
    // The concepts_fts* tables have no foreign key to cascade from, so — now that
    // startup no longer wipes and rebuilds the whole index (issue #295) —
    // deleting a CodeSystem would otherwise orphan its FTS rows and its
    // concepts_fts_built marker. Mirrors the per-system invalidation in
    // write_code_system.
    let system_ids: Vec<String> = conn
        .prepare(
            "SELECT id FROM code_systems \
             WHERE id = ?1 OR json_extract(resource_json, '$.id') = ?1",
        )
        .and_then(|mut s| {
            s.query_map(rusqlite::params![id], |r| r.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    conn.execute(
        "DELETE FROM code_systems \
         WHERE id = ?1 OR json_extract(resource_json, '$.id') = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| HtsError::StorageError(e.to_string()))?;

    for sid in &system_ids {
        let _ = conn.execute(
            "DELETE FROM concepts_fts WHERE system_id = ?1",
            rusqlite::params![sid],
        );
        let _ = conn.execute(
            "DELETE FROM concepts_word_fts WHERE system_id = ?1",
            rusqlite::params![sid],
        );
        let _ = conn.execute(
            "DELETE FROM concepts_search_fts WHERE system_id = ?1",
            rusqlite::params![sid],
        );
        let _ = conn.execute(
            "DELETE FROM concepts_fts_built WHERE system_id = ?1",
            rusqlite::params![sid],
        );
    }

    crate::backends::sqlite::invalidate_cs_id_cache();
    crate::backends::sqlite::invalidate_cs_language_cache();
    Ok(())
}

/// Delete a ValueSet and its materialized expansion cache by FHIR resource `id`.
#[cfg(feature = "sqlite")]
pub(crate) fn delete_value_set(conn: &Connection, id: &str) -> Result<(), HtsError> {
    // Multi-version: match the plain FHIR id *and* every synthetic storage id
    // derived from it. `write_value_set` mints `<fhir-id>|<version>` via
    // `storage_id_for`, so matching only `id = ?1` — as this did — silently
    // no-ops for any ValueSet that carries a `version`. Two consequences, both
    // reachable from the REST API: `DELETE /ValueSet/{id}` returned 204 while
    // leaving the ValueSet fully expandable, and `PUT /ValueSet/{id}` skipped
    // the delete-then-reimport, so the FK cascade never fired and
    // `value_set_expansions` kept serving the pre-update codes. The existing
    // round-trip tests missed both because their ValueSet fixtures carry no
    // `version`.
    //
    // The second predicate compares the segment before the first `|` for exact
    // equality rather than using `LIKE '<id>|%'`, so an id containing a LIKE
    // wildcard cannot over-match, and no escaping is required. FHIR ids cannot
    // contain `|`, so the split is unambiguous.
    //
    // Deliberately NOT matched: `json_extract(resource_json, '$.id')`, which is
    // what `delete_code_system` uses. ValueSets legitimately share a FHIR id
    // across *different* canonical URLs (the tx-ecosystem fixtures do this, and
    // `write_value_set` mints a UUID storage id for the collision), so matching
    // on the embedded id would delete unrelated ValueSets. The remaining gap —
    // a ValueSet stored under such a minted UUID is still not reachable by id —
    // needs URL-based resolution and is out of scope here.
    conn.execute(
        "DELETE FROM value_sets \
         WHERE id = ?1 \
            OR (instr(id, '|') > 0 AND substr(id, 1, instr(id, '|') - 1) = ?1)",
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

/// Build a multi-version-safe storage id for a CodeSystem.
///
/// The HTS schema permits multiple `code_systems` rows that share a canonical
/// `url` provided each row has a distinct `version`. Tx-ecosystem fixtures
/// frequently ship the same FHIR `id` (e.g. `"version"`) for every version of
/// a CodeSystem, so a 1:1 use of `id` would collide on the PK. Suffixing the
/// version makes the storage id deterministic per (url, version) without
/// forcing callers to thread the URL through.
pub(crate) fn storage_id_for(fhir_id: &str, version: Option<&str>) -> String {
    match version {
        Some(v) if !v.is_empty() => format!("{fhir_id}|{v}"),
        _ => fhir_id.to_owned(),
    }
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

    /// #295 (Leg A): re-importing a CodeSystem must invalidate that system's
    /// concept FTS — both the content rows and the `concepts_fts_built` tracker
    /// — so a subsequent build reflects the new data instead of serving stale
    /// display/synonym rows. Proven directly: after a re-import the tracker row
    /// is gone (which forces a rebuild) and the stale display is no longer
    /// indexed; a rebuild then indexes the new display.
    #[tokio::test]
    async fn reimport_invalidates_concept_fts() {
        let b = backend();
        let ctx = ctx();

        // Same id/url/version each time → a stable storage system_id, so the
        // concept is updated in place (the case per-system invalidation covers).
        let cs_bundle = |display: &str| {
            format!(
                r#"{{"resourceType":"Bundle","type":"collection","entry":[
                    {{"resource":{{"resourceType":"CodeSystem","id":"cs-x",
                        "url":"http://example.org/csx","version":"1.0",
                        "status":"active","content":"complete",
                        "concept":[{{"code":"A","display":"{display}"}}]}}}}]}}"#
            )
        };

        let tracker_rows = |b: &SqliteTerminologyBackend| -> i64 {
            let conn = b.pool().get().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM concepts_fts_built cfb \
                 JOIN code_systems cs ON cfb.system_id = cs.id \
                 WHERE cs.url = 'http://example.org/csx'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        let fts_display_count = |b: &SqliteTerminologyBackend, display: &str| -> i64 {
            let conn = b.pool().get().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM concepts_fts WHERE display = ?1",
                [display],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Import + build FTS.
        b.import_bundle(&ctx, cs_bundle("Original A").as_bytes())
            .await
            .unwrap();
        b.finalize_after_bootstrap().unwrap();
        assert_eq!(tracker_rows(&b), 1, "system tracked after first build");
        assert_eq!(
            fts_display_count(&b, "Original A"),
            1,
            "original display indexed"
        );

        // Re-import the same system with a changed display.
        b.import_bundle(&ctx, cs_bundle("Changed A").as_bytes())
            .await
            .unwrap();

        // Leg A must have cleared this system's tracker row AND its FTS content.
        assert_eq!(
            tracker_rows(&b),
            0,
            "re-import must clear the concepts_fts_built tracker row"
        );
        assert_eq!(
            fts_display_count(&b, "Original A"),
            0,
            "stale FTS content must be removed on re-import"
        );

        // A rebuild reflects the new display.
        b.finalize_after_bootstrap().unwrap();
        assert_eq!(tracker_rows(&b), 1, "system re-tracked after rebuild");
        assert_eq!(
            fts_display_count(&b, "Changed A"),
            1,
            "new display indexed after rebuild"
        );
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

    /// Two CodeSystems sharing a canonical URL but declaring distinct
    /// `version` values (and the same FHIR `id`) must coexist.
    ///
    /// Mirrors `tx-ecosystem/tests/version/codesystem-version-{1,2}.json`,
    /// which both ship `"id":"version"` + the same `url`. The legacy
    /// `UNIQUE(url)` constraint dropped one of them; the new composite
    /// `(url, version)` index lets both survive.
    #[tokio::test]
    async fn import_two_versions_same_url_keeps_both() {
        let b = backend();
        let ctx = ctx();

        let bundle = r#"{
          "resourceType": "Bundle",
          "type": "collection",
          "entry": [
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "version",
                "url": "http://example.org/cs/multi",
                "version": "1.0.0",
                "status": "active",
                "content": "complete",
                "concept": [{ "code": "code1", "display": "Display 1 (1.0)" }]
              }
            },
            {
              "resource": {
                "resourceType": "CodeSystem",
                "id": "version",
                "url": "http://example.org/cs/multi",
                "version": "1.2.0",
                "status": "active",
                "content": "complete",
                "concept": [
                  { "code": "code1", "display": "Display 1 (1.2)" },
                  { "code": "code3", "display": "Display 3 (1.2)" }
                ]
              }
            }
          ]
        }"#;

        let stats = b.import_bundle(&ctx, bundle.as_bytes()).await.unwrap();
        assert_eq!(stats.code_systems, 2);
        assert!(
            stats.errors.is_empty(),
            "no errors expected, got: {:?}",
            stats.errors
        );

        let conn = b.pool().get().unwrap();
        let row_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM code_systems WHERE url = 'http://example.org/cs/multi'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(row_count, 2, "both versions must coexist");

        // Each version owns its own concept set.
        let v1_concepts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts c JOIN code_systems s ON c.system_id = s.id \
                 WHERE s.url = 'http://example.org/cs/multi' AND s.version = '1.0.0'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let v2_concepts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts c JOIN code_systems s ON c.system_id = s.id \
                 WHERE s.url = 'http://example.org/cs/multi' AND s.version = '1.2.0'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v1_concepts, 1);
        assert_eq!(v2_concepts, 2);
    }

    #[tokio::test]
    async fn hierarchy_materialized_from_nesting() {
        let b = backend();
        let ctx = ctx();
        b.import_bundle(&ctx, minimal_bundle_json().as_bytes())
            .await
            .unwrap();

        // Multi-version storage_id is opaque, so resolve it via URL first.
        let conn = b.pool().get().unwrap();
        let system_id: String = conn
            .query_row(
                "SELECT id FROM code_systems WHERE url = 'http://example.org/cs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concept_hierarchy WHERE system_id = ?1",
                [&system_id],
                |r| r.get(0),
            )
            .unwrap();
        // A→B and B→C
        assert_eq!(count, 2, "Two hierarchy edges should be materialized");
    }

    /// A bundle with one CodeSystem whose single concept carries the given
    /// designations and properties. Used to verify re-import replacement.
    fn cs_with_concept_extras(designations: &str, properties: &str) -> String {
        format!(
            r#"{{
              "resourceType": "Bundle",
              "type": "collection",
              "entry": [{{
                "resource": {{
                  "resourceType": "CodeSystem",
                  "url": "http://example.org/cs",
                  "version": "1.0",
                  "status": "active",
                  "content": "complete",
                  "concept": [{{
                    "code": "A",
                    "display": "Alpha",
                    "designation": [{designations}],
                    "property": [{properties}]
                  }}]
                }}
              }}]
            }}"#
        )
    }

    fn count_rows(b: &SqliteTerminologyBackend, table: &str) -> i64 {
        let conn = b.pool().get().unwrap();
        let sql = format!(
            "SELECT COUNT(*) FROM {table} d \
             JOIN concepts c ON c.id = d.concept_id \
             JOIN code_systems s ON s.id = c.system_id \
             WHERE s.url = 'http://example.org/cs'"
        );
        conn.query_row(&sql, [], |r| r.get(0)).unwrap()
    }

    /// Re-importing a concept must *replace* its designations and properties,
    /// not accumulate or leave stale rows — even when the new set is smaller or
    /// empty (the filtered-language re-import case). Regression for the
    /// `has_props`/`has_desigs` guards that previously skipped the delete when
    /// the incoming slice was empty.
    #[tokio::test]
    async fn reimport_replaces_designations_and_properties() {
        let b = backend();
        let ctx = ctx();

        // First import: two designations (en + de) and one property.
        let first = cs_with_concept_extras(
            r#"{ "language": "en", "value": "Alpha" },
               { "language": "de", "value": "Alpha-DE" }"#,
            r#"{ "code": "p1", "valueString": "v1" }"#,
        );
        b.import_bundle(&ctx, first.as_bytes()).await.unwrap();
        assert_eq!(count_rows(&b, "concept_designations"), 2);
        assert_eq!(count_rows(&b, "concept_properties"), 1);

        // Re-import with a narrower set: only the English designation, no
        // properties. The German designation and the property must be gone.
        let second = cs_with_concept_extras(r#"{ "language": "en", "value": "Alpha" }"#, "");
        b.import_bundle(&ctx, second.as_bytes()).await.unwrap();
        assert_eq!(
            count_rows(&b, "concept_designations"),
            1,
            "stale German designation must be removed on re-import"
        );
        assert_eq!(
            count_rows(&b, "concept_properties"),
            0,
            "stale property must be removed on re-import"
        );
    }
}
