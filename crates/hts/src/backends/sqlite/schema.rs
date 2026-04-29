//! SQLite DDL and migrations for the HTS terminology schema.
//!
//! # Layout
//!
//! - [`SCHEMA`] — the initial DDL, applied idempotently on every startup via
//!   [`apply`].  Uses `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS`
//!   throughout so repeated application is a no-op.
//! - [`migrate_search_columns`] — additive migration that adds columns and
//!   indexes required by the search handlers to pre-existing databases.
//!
//! Tables model the core FHIR terminology resources — `code_systems`,
//! `concepts`, `concept_hierarchy`, `value_sets`, `value_set_expansions`,
//! `concept_maps`, and their child tables (properties, designations, group
//! elements) — plus `concept_closure` used by `$closure`.

/// SQL DDL for the HTS SQLite schema.
///
/// All statements use `CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS`
/// so this can be applied safely on every startup without error.
pub const SCHEMA: &str = "
-- ── Code Systems ──────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS code_systems (
    id            TEXT PRIMARY KEY,
    url           TEXT NOT NULL UNIQUE,
    version       TEXT,
    name          TEXT,
    title         TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    content       TEXT NOT NULL DEFAULT 'complete',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    resource_json TEXT
);

-- ── Concepts ───────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS concepts (
    id          INTEGER PRIMARY KEY,
    system_id   TEXT NOT NULL REFERENCES code_systems(id) ON DELETE CASCADE,
    code        TEXT NOT NULL,
    display     TEXT,
    definition  TEXT,
    UNIQUE(system_id, code)
);
CREATE INDEX IF NOT EXISTS idx_concepts_system_code ON concepts(system_id, code);

-- ── Hierarchy (pre-materialized parent-child links) ───────────────────────────
CREATE TABLE IF NOT EXISTS concept_hierarchy (
    system_id   TEXT NOT NULL REFERENCES code_systems(id) ON DELETE CASCADE,
    parent_code TEXT NOT NULL,
    child_code  TEXT NOT NULL,
    PRIMARY KEY (system_id, parent_code, child_code)
);
CREATE INDEX IF NOT EXISTS idx_hierarchy_child ON concept_hierarchy(system_id, child_code);

-- ── Concept Properties (arbitrary FHIR properties) ────────────────────────────
CREATE TABLE IF NOT EXISTS concept_properties (
    id          INTEGER PRIMARY KEY,
    concept_id  INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    property    TEXT NOT NULL,
    value_type  TEXT NOT NULL,
    value       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_concept_properties_lookup
    ON concept_properties(concept_id, property, value);
-- Reverse index: supports property=value filter queries (e.g. EX06 tradename_of).
-- The forward index starts with concept_id and cannot serve property-value lookups.
CREATE INDEX IF NOT EXISTS idx_concept_properties_value
    ON concept_properties(property, value, concept_id);

-- ── Designations (alternate names / translations) ─────────────────────────────
CREATE TABLE IF NOT EXISTS concept_designations (
    id          INTEGER PRIMARY KEY,
    concept_id  INTEGER NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
    language    TEXT,
    use_system  TEXT,
    use_code    TEXT,
    value       TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_code_systems_created_at ON code_systems(created_at);
-- Covering index for metadata-only list queries (summary=true / _count without resource_json).
-- Allows ORDER BY created_at LIMIT N to be served entirely from the index,
-- with no main B-tree access for the large resource_json column.
CREATE INDEX IF NOT EXISTS idx_code_systems_meta
    ON code_systems(created_at, id, url, version, name, title, status);

-- ── Value Sets ─────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS value_sets (
    id            TEXT PRIMARY KEY,
    url           TEXT NOT NULL UNIQUE,
    version       TEXT,
    name          TEXT,
    title         TEXT,
    status        TEXT NOT NULL DEFAULT 'active',
    compose_json  TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    resource_json TEXT
);
CREATE INDEX IF NOT EXISTS idx_value_sets_created_at ON value_sets(created_at);
-- Covering index for metadata-only list queries (analogous to idx_code_systems_meta).
CREATE INDEX IF NOT EXISTS idx_value_sets_meta
    ON value_sets(created_at, id, url, version, name, title, status);

-- ── Value Set Expansions (materialized cache) ─────────────────────────────────
CREATE TABLE IF NOT EXISTS value_set_expansions (
    value_set_id TEXT NOT NULL REFERENCES value_sets(id) ON DELETE CASCADE,
    system_url   TEXT NOT NULL,
    code         TEXT NOT NULL,
    display      TEXT,
    PRIMARY KEY (value_set_id, system_url, code)
);

-- ── Implicit expansion cache ───────────────────────────────────────────────────
-- Caches expansions for implicit ValueSet URLs (e.g. ?fhir_vs patterns) that
-- have no corresponding row in value_sets. Keyed by the full URL string.
CREATE TABLE IF NOT EXISTS implicit_expansion_cache (
    url        TEXT NOT NULL,
    system_url TEXT NOT NULL,
    code       TEXT NOT NULL,
    display    TEXT,
    PRIMARY KEY (url, system_url, code)
);
CREATE INDEX IF NOT EXISTS idx_implicit_expansion_cache_url
    ON implicit_expansion_cache(url);
CREATE INDEX IF NOT EXISTS idx_implicit_expansion_cache_url_code
    ON implicit_expansion_cache(url, code);

-- ── Concept Maps ───────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS concept_maps (
    id          TEXT PRIMARY KEY,
    url         TEXT NOT NULL UNIQUE,
    version     TEXT,
    source_uri  TEXT,
    target_uri  TEXT,
    status      TEXT NOT NULL DEFAULT 'active',
    created_at  TEXT NOT NULL
);

-- ── Concept Map Elements ───────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS concept_map_elements (
    id            INTEGER PRIMARY KEY,
    map_id        TEXT NOT NULL REFERENCES concept_maps(id) ON DELETE CASCADE,
    source_system TEXT NOT NULL,
    source_code   TEXT NOT NULL,
    target_system TEXT NOT NULL,
    target_code   TEXT NOT NULL,
    equivalence   TEXT NOT NULL DEFAULT 'equivalent'
);
CREATE INDEX IF NOT EXISTS idx_map_source
    ON concept_map_elements(map_id, source_system, source_code);
-- Forward and reverse lookup by code (without knowing map_id first).
-- Needed when the caller knows only the source/target code and optionally system.
CREATE INDEX IF NOT EXISTS idx_map_elements_source_code
    ON concept_map_elements(source_code, source_system, map_id);
CREATE INDEX IF NOT EXISTS idx_map_elements_target_code
    ON concept_map_elements(target_code, target_system, map_id);

-- ── FTS5 trigram index for implicit expansion text search ─────────────────────
-- Enables fast substring matching on code and display in implicit_expansion_cache.
-- url and system_url are UNINDEXED (stored, not tokenised).
-- case_sensitive=0 makes queries match regardless of case.
-- Requires SQLite ≥ 3.38 with FTS5 (provided by the bundled rusqlite feature).
CREATE VIRTUAL TABLE IF NOT EXISTS implicit_expansion_fts
USING fts5(url UNINDEXED, system_url UNINDEXED, code, display,
           tokenize='trigram case_sensitive 0');

-- ── FTS5 trigram index for direct concept text search ─────────────────────────
-- Enables fast substring matching on concepts.code and concepts.display.
-- Used by expand_inline_filtered for full-system includes with a text filter.
-- system_id is UNINDEXED: stored for post-filter, not tokenised.
-- Populated lazily per system_id on first filtered expand; cleared on startup.
CREATE VIRTUAL TABLE IF NOT EXISTS concepts_fts
USING fts5(system_id UNINDEXED, code, display,
           tokenize='trigram case_sensitive 0');

-- ── FTS build tracker ─────────────────────────────────────────────────────────
-- O(1) lookup to check whether concepts_fts is populated for a given system_id.
-- Replaces the slow FTS content scan (O(N_total_concepts)) used previously.
-- Cleared on startup alongside concepts_fts; populated in ensure_concepts_fts
-- and prebuild_concepts_fts.
CREATE TABLE IF NOT EXISTS concepts_fts_built (
    system_id TEXT PRIMARY KEY
);

-- ── Transitive ancestor closure ───────────────────────────────────────────────
-- Precomputed (ancestor, descendant) pairs for every code system, including
-- self-links (code, code).  Populated at import time for each code system so
-- that is-a, descendent-of, generalizes, and $subsumes queries are O(1) index
-- lookups rather than O(depth) recursive CTEs at request time.
CREATE TABLE IF NOT EXISTS concept_closure (
    system_id       TEXT NOT NULL,
    ancestor_code   TEXT NOT NULL,
    descendant_code TEXT NOT NULL,
    PRIMARY KEY (system_id, ancestor_code, descendant_code)
);
-- Reverse lookup: all ancestors of a given descendant code.
CREATE INDEX IF NOT EXISTS idx_closure_descendant
    ON concept_closure(system_id, descendant_code);
";

/// Apply the HTS schema to the given database connection.
///
/// Safe to call on every startup — all statements are idempotent.
pub fn apply(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

/// Build (or rebuild) the transitive ancestor closure for one code system.
///
/// Deletes any existing closure rows for `system_id`, then recomputes the
/// full set of `(ancestor, descendant)` pairs — including self-links
/// `(code, code)` — in a single recursive SQL pass.  The UNION (not UNION
/// ALL) prevents path explosion in SNOMED polyhierarchies.
///
/// Runs inside whatever transaction the caller has open; call after all
/// `concept_hierarchy` rows for the system have been inserted.
pub fn build_concept_closure(conn: &rusqlite::Connection, system_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM concept_closure WHERE system_id = ?1",
        rusqlite::params![system_id],
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO concept_closure (system_id, ancestor_code, descendant_code)
         WITH RECURSIVE closure(anc, desc) AS (
             SELECT code, code FROM concepts WHERE system_id = ?1
             UNION
             SELECT c.anc, h.child_code
             FROM   closure c
             JOIN   concept_hierarchy h
                    ON h.parent_code = c.desc AND h.system_id = ?1
         )
         SELECT ?1, anc, desc FROM closure",
        rusqlite::params![system_id],
    )?;

    Ok(())
}

/// Populate `concept_closure` for all code systems that currently have
/// hierarchy edges but no closure rows.
///
/// Called once at startup so that existing databases (imported before the
/// closure table was introduced) are migrated automatically.
pub fn migrate_concept_closure(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let has_hierarchy: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM concept_hierarchy LIMIT 1)",
        [],
        |r| r.get(0),
    )?;

    if !has_hierarchy {
        return Ok(());
    }

    let closure_populated: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM concept_closure LIMIT 1)",
        [],
        |r| r.get(0),
    )?;

    if closure_populated {
        return Ok(());
    }

    // No closure yet — build for every code system with hierarchy data.
    let system_ids: Vec<String> = {
        let mut stmt = conn.prepare("SELECT DISTINCT system_id FROM concept_hierarchy")?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };

    for sid in &system_ids {
        build_concept_closure(conn, sid)?;
    }

    Ok(())
}

/// Add search-related columns to the existing tables.
///
/// `title` and `resource_json` are added to all three resource tables.
/// `name` is added to `concept_maps` (it was absent from the original schema).
///
/// Uses `ALTER TABLE … ADD COLUMN` and silently ignores
/// "duplicate column name" errors so this is safe to run on every startup.
pub fn migrate_search_columns(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let migrations = [
        "ALTER TABLE code_systems ADD COLUMN title TEXT",
        "ALTER TABLE code_systems ADD COLUMN resource_json TEXT",
        "ALTER TABLE value_sets ADD COLUMN title TEXT",
        "ALTER TABLE value_sets ADD COLUMN resource_json TEXT",
        "ALTER TABLE concept_maps ADD COLUMN name TEXT",
        "ALTER TABLE concept_maps ADD COLUMN title TEXT",
        "ALTER TABLE concept_maps ADD COLUMN resource_json TEXT",
    ];
    for sql in &migrations {
        match conn.execute_batch(sql) {
            Ok(_) => {}
            // SQLite error 1 with "duplicate column name" means the column already
            // exists — skip silently so this migration is idempotent.
            Err(e) if e.to_string().contains("duplicate column name") => {}
            Err(e) => return Err(e),
        }
    }

    // Idempotent index additions (IF NOT EXISTS handles repeated runs).
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_concept_properties_value
             ON concept_properties(property, value, concept_id);
         CREATE INDEX IF NOT EXISTS idx_map_elements_source_code
             ON concept_map_elements(source_code, source_system, map_id);
         CREATE INDEX IF NOT EXISTS idx_map_elements_target_code
             ON concept_map_elements(target_code, target_system, map_id);
         CREATE INDEX IF NOT EXISTS idx_code_systems_created_at
             ON code_systems(created_at);
         CREATE INDEX IF NOT EXISTS idx_value_sets_created_at
             ON value_sets(created_at);
         CREATE INDEX IF NOT EXISTS idx_code_systems_meta
             ON code_systems(created_at, id, url, version, name, title, status);
         CREATE INDEX IF NOT EXISTS idx_value_sets_meta
             ON value_sets(created_at, id, url, version, name, title, status);",
    )?;

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_applies_to_in_memory_db() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply(&conn).expect("schema should apply without error");
    }

    #[test]
    fn schema_is_idempotent() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply(&conn).expect("first application should succeed");
        apply(&conn).expect("second application should also succeed (idempotent)");
    }

    #[test]
    fn all_tables_exist_after_migration() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();

        let expected_tables = [
            "code_systems",
            "concepts",
            "concept_hierarchy",
            "concept_closure",
            "concept_properties",
            "concept_designations",
            "value_sets",
            "value_set_expansions",
            "concept_maps",
            "concept_map_elements",
            "implicit_expansion_cache",
            "implicit_expansion_fts",
        ];

        for table in &expected_tables {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    rusqlite::params![table],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            assert_eq!(count, 1, "table '{table}' should exist after migration");
        }
    }

    #[test]
    fn foreign_key_cascade_works() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        // Insert a code system
        conn.execute(
            "INSERT INTO code_systems (id, url, status, content, created_at, updated_at)
             VALUES ('cs1', 'http://example.org/cs', 'active', 'complete', '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();

        // Insert a concept in that system
        conn.execute(
            "INSERT INTO concepts (system_id, code, display) VALUES ('cs1', 'A', 'Alpha')",
            [],
        )
        .unwrap();

        // Deleting the code system should cascade to concepts
        conn.execute("DELETE FROM code_systems WHERE id='cs1'", [])
            .unwrap();

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE system_id='cs1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "cascade delete should remove child concepts");
    }
}
